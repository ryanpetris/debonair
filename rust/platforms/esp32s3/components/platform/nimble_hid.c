// HID over GATT peripheral on NimBLE: presents the Dongle to Targets as a Bluetooth LE keyboard
// and/or mouse. Report IDs, reports and pairing rules match the BTstack adapter.
#include "nimble_hid.h"
#include <string.h>
#include "freertos/FreeRTOS.h"
#include "freertos/queue.h"
#include "host/ble_gap.h"
#include "host/ble_gatt.h"
#include "host/ble_hs.h"
#include "host/ble_hs_mbuf.h"
#include "host/ble_sm.h"
#include "host/ble_store.h"
#include "nimble/nimble_port.h"
#include "nimble/nimble_port_freertos.h"
#include "os/os_mbuf.h"
#include "services/gap/ble_svc_gap.h"
#include "services/gatt/ble_svc_gatt.h"
#include "store/config/ble_store_config.h"

void ble_store_config_init(void);
// Exported by NimBLE but declared only in its private headers. Removes a peer's identity key from
// the controller's resolving list, which ble_store_util_delete_peer leaves in place.
int ble_hs_pvcy_remove_entry(uint8_t addr_type, const uint8_t *addr);

_Static_assert(MYNEWT_VAL(BLE_HS_PVCY), "identity keys require NimBLE privacy support");
_Static_assert(MYNEWT_VAL(BLE_STORE_MAX_BONDS) == DEBONAIR_BONDS, "the pairing records hold 8 bonds");

#define NAME_BYTES 29
#define DESCRIPTOR_BYTES 512
#define BONDS DEBONAIR_BONDS
// How long a Target may stay connected without encrypting. Pairing with a passkey needs time to
// type it.
#define UNENCRYPTED_PAIRING_MS 60000
#define UNENCRYPTED_MS 20000
#define RETRY_MS 5
// Reports wait in NimBLE's buffers only while this many stay free for everything else, so a busy
// connection backs up into the application, which merges pointer motion.
#define FREE_BUFFERS 6

// ---- State ---------------------------------------------------------------------------------
// Fields marked "host" are used only on the NimBLE host task.

static debonair_emit emit;
static char name[NAME_BYTES + 1];          // host
static uint8_t name_length;                // host
static uint8_t descriptor[DESCRIPTOR_BYTES];
static uint16_t descriptor_length;
static char manufacturer[32];
static char model[65];
static char firmware[48];
static uint8_t pnp[7];
static uint16_t appearance;
static uint8_t identity[6];                // least significant byte first, as NimBLE stores it
static uint8_t local_irk[16];             // least significant byte first
// Identifies this firmware's database, so a paired Target is told to rediscover after it changes.
static uint32_t layout;
static bool pairing;                       // host
static bool passkey_mode;                  // host
static volatile uint16_t connection = BLE_HS_CONN_HANDLE_NONE;
static volatile bool encrypted;
static volatile bool want_send;
// Keys written during this connection: a new pairing. Refused: a pairing this mode does not accept,
// whose leftover records are removed when the Target disconnects.
static bool wrote_keys;                    // host
static bool refused;                       // host
static ble_addr_t refused_peer;            // host
static bool refused_known;                 // host: refused_peer holds the refused identity
static bool ready_sent;                    // host
static uint8_t protocol_mode = 1;          // host
static uint8_t leds;                       // host
static uint8_t feature;                    // host
static uint8_t battery;                    // host
static uint8_t last_keyboard[8];           // host
static uint8_t last_mouse[9];              // host
static uint8_t last_consumer[4];           // host
static uint16_t service_changed_handle;    // host
static uint16_t keyboard_handle, mouse_handle, consumer_handle;
static uint16_t boot_keyboard_handle, boot_mouse_handle, battery_handle;

typedef struct {
    uint8_t channel;
    uint8_t report_id;
    uint8_t length;
    uint8_t data[9];
} report;

static QueueHandle_t commands;
static QueueHandle_t reports;
static struct ble_npl_event command_event;
static struct ble_npl_event report_event;
static struct ble_npl_callout encryption_timer;
static struct ble_npl_callout retry_timer;

static void send_event(uint8_t kind, uint8_t code, uint8_t report_id, const ble_addr_t *address,
                       const uint8_t *data, uint16_t length) {
    debonair_event event = {
        .kind = kind, .code = code, .report_id = report_id, .entry = -1, .length = length, .data = data};
    if (address) {
        event.code = address->type & 1;
        for (int i = 0; i < 6; i++) event.address[i] = address->val[5 - i];
    }
    emit(&event);
}

static void simple_event(uint8_t kind) { send_event(kind, 0, 0, NULL, NULL, 0); }

// ---- Pairing records -------------------------------------------------------------------------
// NimBLE reads and writes keys through these callbacks on the host task. Records live in RAM, in
// the form the application saves; the application imports them before NimBLE starts, and
// BONDS_CHANGED tells it to take the changes and save them. Each device combination has its own
// records, so its pairings stay apart.
//
// A record is a complete pairing once the Target's keys are written; until then it holds an
// incomplete pairing, which the application keeps unsaved. A paired Target pairing again has its
// record marked incomplete before either side's keys are deleted, and it stays incomplete until the
// new pairing's keys are written, so the saved bond stays until the new pairing is saved. A record
// still incomplete when the Target disconnects is emptied.

typedef struct {
    bool used;
    // The keys were cleared for a new pairing of the same Target, which the next key write starts.
    bool fresh;
    uint32_t age;   // higher is newer
    debonair_bond bond;
} record;

_Static_assert(sizeof(debonair_keys) == 64, "debonair_keys layout");
_Static_assert(sizeof(debonair_bond) == 188, "debonair_bond layout");

static record records[BONDS];
static uint32_t newest;
static uint32_t changed_slots, added_slots, updated_slots;
static portMUX_TYPE bonds_lock = portMUX_INITIALIZER_UNLOCKED;
// Set while a paired Target's keys are deleted so that it can pair again.
static bool repairing;                     // host
static uint32_t forgotten;

// NimBLE keeps keys and addresses least significant byte first; the records keep them most
// significant byte first.
static void reverse(uint8_t *to, const uint8_t *from, int length) {
    for (int i = 0; i < length; i++) to[i] = from[length - 1 - i];
}

static void pack(debonair_bond *b, debonair_keys *k, const struct ble_store_value_sec *v) {
    memset(k, 0, sizeof *k);
    k->flags = DEBONAIR_KEYS_PRESENT | (v->ltk_present ? DEBONAIR_KEY_LTK : 0) |
               (v->irk_present ? DEBONAIR_KEY_IRK : 0) | (v->csrk_present ? DEBONAIR_KEY_CSRK : 0);
    k->ediv = v->ediv;
    for (int i = 0; i < 8; i++) k->rand[i] = v->rand_num >> (8 * (7 - i));
    reverse(k->ltk, v->ltk, 16);
    reverse(k->irk, v->irk, 16);
    reverse(k->csrk, v->csrk, 16);
    k->sign_counter = v->sign_counter;
    b->key_size = v->key_size;
    b->flags = (b->flags & (DEBONAIR_BOND_FEATURE | DEBONAIR_BOND_AUTHORIZED)) |
               (v->authenticated ? DEBONAIR_BOND_AUTHENTICATED : 0) |
               (v->sc ? DEBONAIR_BOND_SECURE_CONNECTIONS : 0);
}

static void unpack(struct ble_store_value_sec *v, const debonair_bond *b, const debonair_keys *k) {
    memset(v, 0, sizeof *v);
    v->peer_addr.type = b->random;
    reverse(v->peer_addr.val, b->address, 6);
    v->ltk_present = (k->flags & DEBONAIR_KEY_LTK) != 0;
    v->irk_present = (k->flags & DEBONAIR_KEY_IRK) != 0;
    v->csrk_present = (k->flags & DEBONAIR_KEY_CSRK) != 0;
    v->authenticated = (b->flags & DEBONAIR_BOND_AUTHENTICATED) != 0;
    v->sc = (b->flags & DEBONAIR_BOND_SECURE_CONNECTIONS) != 0;
    v->key_size = b->key_size;
    v->ediv = k->ediv;
    for (int i = 0; i < 8; i++) v->rand_num = (v->rand_num << 8) | k->rand[i];
    reverse(v->ltk, k->ltk, 16);
    reverse(v->irk, k->irk, 16);
    reverse(v->csrk, k->csrk, 16);
    v->sign_counter = k->sign_counter;
}

static bool matches(const record *r, const ble_addr_t *address) {
    uint8_t value[6];
    reverse(value, address->val, 6);
    return r->used && r->bond.random == (address->type & 1) && memcmp(r->bond.address, value, 6) == 0;
}

static int find(const ble_addr_t *address) {
    for (int i = 0; i < BONDS; i++) {
        if (matches(&records[i], address)) return i;
    }
    return -1;
}

static bool complete(const record *r) {
    return r->used && !r->fresh && (r->bond.peer.flags & DEBONAIR_KEYS_PRESENT);
}

// The record holding a complete pairing with `address`, or -1.
static int find_paired(const ble_addr_t *address) {
    int slot = find(address);
    return slot >= 0 && complete(&records[slot]) ? slot : -1;
}

// Starts a new pairing in `slot`.
static void start_record(int slot, const ble_addr_t *address) {
    record *r = &records[slot];
    memset(r, 0, sizeof *r);
    r->used = true;
    r->age = ++newest;
    r->bond.random = address->type & 1;
    reverse(r->bond.address, address->val, 6);
    // The Target discovers this database while pairing.
    memcpy(r->bond.database, &layout, sizeof layout);
    r->bond.database_length = sizeof layout;
    added_slots |= 1u << slot;
}

// Deletes a paired Target's keys so that it can pair again, keeping its record as an incomplete
// pairing from before the first side's keys go.
static void delete_for_pairing(const ble_addr_t *address) {
    portENTER_CRITICAL(&bonds_lock);
    int slot = find(address);
    if (slot >= 0) records[slot].fresh = true;
    portEXIT_CRITICAL(&bonds_lock);
    repairing = true;
    ble_store_util_delete_peer(address);
    repairing = false;
}

static bool security_type(int type) {
    return type == BLE_STORE_OBJ_TYPE_OUR_SEC || type == BLE_STORE_OBJ_TYPE_PEER_SEC;
}

static debonair_keys *keys_of(record *r, int type) {
    return type == BLE_STORE_OBJ_TYPE_OUR_SEC ? &r->bond.local : &r->bond.peer;
}

// Bonds are listed oldest first, so NimBLE's round-robin overflow handling removes the oldest.
static int store_read(int type, const union ble_store_key *key, union ble_store_value *value) {
    if (!security_type(type)) return ble_store_config_read(type, key, value);
    bool any = ble_addr_cmp(&key->sec.peer_addr, BLE_ADDR_ANY) == 0;
    int skip = key->sec.idx;
    int result = BLE_HS_ENOENT;
    portENTER_CRITICAL(&bonds_lock);
    uint32_t after = 0;
    for (;;) {
        // The next oldest bond after `after`.
        record *next = NULL;
        for (int i = 0; i < BONDS; i++) {
            record *r = &records[i];
            if (!r->used || r->fresh || !(keys_of(r, type)->flags & DEBONAIR_KEYS_PRESENT) || r->age < after) {
                continue;
            }
            if (!next || r->age < next->age) next = r;
        }
        if (!next) break;
        after = next->age + 1;
        if (!any && !matches(next, &key->sec.peer_addr)) continue;
        if (skip-- > 0) continue;
        unpack(&value->sec, &next->bond, keys_of(next, type));
        result = 0;
        break;
    }
    portEXIT_CRITICAL(&bonds_lock);
    return result;
}

static int store_write(int type, const union ble_store_value *value) {
    if (!security_type(type)) return ble_store_config_write(type, value);
    const struct ble_store_value_sec *v = &value->sec;
    // NimBLE writes keys only for a pairing. Only pairing mode accepts one, even for an address
    // that is already paired, and passkey mode only one that entered the passkey.
    if (!pairing || (passkey_mode && !v->authenticated)) {
        refused = true;
        return BLE_HS_EREJECT;
    }
    int result = 0;
    portENTER_CRITICAL(&bonds_lock);
    int slot = find(&v->peer_addr);
    if (slot >= 0 && records[slot].fresh) {
        start_record(slot, &v->peer_addr);
    } else if (slot < 0) {
        for (int i = 0; i < BONDS && slot < 0; i++) {
            if (!records[i].used) slot = i;
        }
        if (slot >= 0) start_record(slot, &v->peer_addr);
    }
    if (slot >= 0) {
        debonair_keys *keys = keys_of(&records[slot], type);
        if (keys->flags & DEBONAIR_KEYS_PRESENT) {
            // Keys written over kept keys: the Target paired again, so this is the newest pairing.
            records[slot].age = ++newest;
            added_slots |= 1u << slot;
        }
        pack(&records[slot].bond, keys, v);
        changed_slots |= 1u << slot;
    } else {
        result = BLE_HS_ESTORE_CAP;
    }
    portEXIT_CRITICAL(&bonds_lock);
    if (result == 0) {
        wrote_keys = true;
        simple_event(DEBONAIR_EVENT_BONDS_CHANGED);
    }
    return result;
}

static int store_delete(int type, const union ble_store_key *key) {
    if (!security_type(type)) return ble_store_config_delete(type, key);
    int result = BLE_HS_ENOENT;
    portENTER_CRITICAL(&bonds_lock);
    int slot = find(&key->sec.peer_addr);
    if (slot >= 0) {
        record *r = &records[slot];
        debonair_keys *keys = keys_of(r, type);
        if (keys->flags & DEBONAIR_KEYS_PRESENT) {
            memset(keys, 0, sizeof *keys);
            // A Target pairing again keeps its record, already marked incomplete.
            if (!repairing && !(r->bond.local.flags & DEBONAIR_KEYS_PRESENT) &&
                !(r->bond.peer.flags & DEBONAIR_KEYS_PRESENT)) {
                memset(r, 0, sizeof *r);
            }
            changed_slots |= 1u << slot;
            result = 0;
        }
    }
    portEXIT_CRITICAL(&bonds_lock);
    if (result == 0) simple_event(DEBONAIR_EVENT_BONDS_CHANGED);
    return result;
}

/** Makes room for a new pairing by removing the oldest, but never removes a pairing to make room
 * for other records, such as those a refused pairing leaves behind. */
static int store_status(struct ble_store_status_event *event, void *arg) {
    if (event->event_code == BLE_STORE_EVENT_OVERFLOW && security_type(event->overflow.obj_type)) {
        return ble_store_util_status_rr(event, arg);
    }
    return event->event_code == BLE_STORE_EVENT_FULL ? 0 : BLE_HS_ESTORE_CAP;
}

static int generate_key(uint8_t kind, struct ble_store_gen_key *key, uint16_t handle) {
    (void)handle;
    if (kind != BLE_STORE_GEN_KEY_IRK) return BLE_HS_ENOTSUP;
    memcpy(key->irk, local_irk, 16);
    return 0;
}

void debonair_nimble_bonds_reset(void) {
    memset(records, 0, sizeof records);
    newest = 0;
    changed_slots = added_slots = updated_slots = 0;
}

int debonair_nimble_bonds_import(const debonair_bond *bond) {
    for (int i = 0; i < BONDS; i++) {
        if (records[i].used) continue;
        records[i].used = true;
        records[i].age = ++newest;
        records[i].bond = *bond;
        return i;
    }
    return -1;
}

int debonair_nimble_bonds_export(int slot, debonair_bond *out) {
    if (slot < 0 || slot >= BONDS) return 0;
    portENTER_CRITICAL(&bonds_lock);
    int result = 0;
    if (records[slot].used) {
        *out = records[slot].bond;
        result = complete(&records[slot]) ? 1 : 2;
    }
    portEXIT_CRITICAL(&bonds_lock);
    return result;
}

uint32_t debonair_nimble_bonds_take(uint32_t *added, uint32_t *updated) {
    portENTER_CRITICAL(&bonds_lock);
    uint32_t changed = changed_slots;
    *added = added_slots;
    *updated = updated_slots;
    changed_slots = added_slots = updated_slots = 0;
    portEXIT_CRITICAL(&bonds_lock);
    return changed;
}

int debonair_nimble_targets(uint8_t *out, int capacity) {
    int count = 0;
    portENTER_CRITICAL(&bonds_lock);
    for (int i = 0; i < BONDS && count < capacity; i++) {
        const record *r = &records[i];
        if (!complete(r)) continue;
        out[count * 7] = r->bond.random;
        memcpy(&out[count * 7 + 1], r->bond.address, 6);
        count++;
    }
    portEXIT_CRITICAL(&bonds_lock);
    return count;
}

int debonair_nimble_max_targets(void) { return BONDS; }

/** Remembers the feature report for the connected Target, which sets it only when it first sees
 * the device: Windows and Linux keep the HID device, and its multipliers, across reconnections. */
static void save_feature(void) {
    struct ble_gap_conn_desc desc;
    if (connection == BLE_HS_CONN_HANDLE_NONE || ble_gap_conn_find(connection, &desc) != 0) return;
    bool saved = false;
    portENTER_CRITICAL(&bonds_lock);
    int slot = find(&desc.peer_id_addr);
    if (slot >= 0) {
        debonair_bond *b = &records[slot].bond;
        if (!(b->flags & DEBONAIR_BOND_FEATURE) || b->feature != feature) {
            b->flags |= DEBONAIR_BOND_FEATURE;
            b->feature = feature;
            updated_slots |= 1u << slot;
            saved = true;
        }
    }
    portEXIT_CRITICAL(&bonds_lock);
    if (saved) simple_event(DEBONAIR_EVENT_BONDS_CHANGED);
}

static bool layout_is(const debonair_bond *b) {
    return b->database_length == sizeof layout && memcmp(b->database, &layout, sizeof layout) == 0;
}

static void save_layout(void) {
    struct ble_gap_conn_desc desc;
    if (connection == BLE_HS_CONN_HANDLE_NONE || ble_gap_conn_find(connection, &desc) != 0) return;
    bool saved = false;
    portENTER_CRITICAL(&bonds_lock);
    int slot = find(&desc.peer_id_addr);
    if (slot >= 0 && !layout_is(&records[slot].bond)) {
        debonair_bond *b = &records[slot].bond;
        memset(b->database, 0, sizeof b->database);
        memcpy(b->database, &layout, sizeof layout);
        b->database_length = sizeof layout;
        updated_slots |= 1u << slot;
        saved = true;
    }
    portEXIT_CRITICAL(&bonds_lock);
    if (saved) simple_event(DEBONAIR_EVENT_BONDS_CHANGED);
}

/** Restores what a paired Target set before, and tells it to rediscover services when this
 * firmware's database differs from the one it cached. */
static void restore_bond(const ble_addr_t *address) {
    bool restore = false;
    bool changed = false;
    portENTER_CRITICAL(&bonds_lock);
    int slot = find(address);
    if (slot >= 0) {
        const debonair_bond *b = &records[slot].bond;
        restore = (b->flags & DEBONAIR_BOND_FEATURE) != 0;
        if (restore) feature = b->feature;
        changed = !layout_is(b);
    }
    portEXIT_CRITICAL(&bonds_lock);
    if (restore) send_event(DEBONAIR_EVENT_FEATURE, 0, 2, NULL, &feature, 1);
    if (changed && service_changed_handle) {
        static const uint8_t range[] = {0x01, 0x00, 0xff, 0xff};
        struct os_mbuf *om = ble_hs_mbuf_from_flat(range, sizeof range);
        // Saved once the Target confirms, so an unconfirmed change is sent again next time.
        if (om) ble_gatts_indicate_custom(connection, service_changed_handle, om);
    }
}

// ---- GATT ----------------------------------------------------------------------------------

enum {
    ATTR_PROTOCOL_MODE = 1,
    ATTR_KEYBOARD,
    ATTR_LEDS,
    ATTR_CONSUMER,
    ATTR_MOUSE,
    ATTR_FEATURE,
    ATTR_REPORT_MAP,
    ATTR_BOOT_KEYBOARD,
    ATTR_BOOT_KEYBOARD_OUTPUT,
    ATTR_BOOT_MOUSE,
    ATTR_INFORMATION,
    ATTR_CONTROL_POINT,
    ATTR_BATTERY,
    ATTR_MANUFACTURER,
    ATTR_MODEL,
    ATTR_FIRMWARE,
    ATTR_PNP,
};

static int append(struct ble_gatt_access_ctxt *ctxt, const void *data, uint16_t length) {
    return os_mbuf_append(ctxt->om, data, length) == 0 ? 0 : BLE_ATT_ERR_INSUFFICIENT_RES;
}

static int access_attribute(uint16_t handle, uint16_t attribute, struct ble_gatt_access_ctxt *ctxt, void *arg) {
    (void)attribute;
    int id = (int)(intptr_t)arg;
    if (ctxt->op == BLE_GATT_ACCESS_OP_READ_CHR) {
        static const uint8_t information[] = {0x11, 0x01, 0x00, 0x02};
        switch (id) {
        case ATTR_PROTOCOL_MODE: return append(ctxt, &protocol_mode, 1);
        case ATTR_KEYBOARD:
        case ATTR_BOOT_KEYBOARD: return append(ctxt, last_keyboard, sizeof last_keyboard);
        case ATTR_LEDS:
        case ATTR_BOOT_KEYBOARD_OUTPUT: return append(ctxt, &leds, 1);
        case ATTR_CONSUMER: return append(ctxt, last_consumer, sizeof last_consumer);
        case ATTR_MOUSE: return append(ctxt, last_mouse, sizeof last_mouse);
        case ATTR_BOOT_MOUSE: return append(ctxt, last_mouse, 3);
        case ATTR_FEATURE: return append(ctxt, &feature, 1);
        case ATTR_REPORT_MAP: return append(ctxt, descriptor, descriptor_length);
        case ATTR_INFORMATION: return append(ctxt, information, sizeof information);
        case ATTR_BATTERY: return append(ctxt, &battery, 1);
        case ATTR_MANUFACTURER: return append(ctxt, manufacturer, strlen(manufacturer));
        case ATTR_MODEL: return append(ctxt, model, strlen(model));
        case ATTR_FIRMWARE: return append(ctxt, firmware, strlen(firmware));
        case ATTR_PNP: return append(ctxt, pnp, sizeof pnp);
        default: return BLE_ATT_ERR_UNLIKELY;
        }
    }
    if (ctxt->op == BLE_GATT_ACCESS_OP_WRITE_CHR) {
        uint8_t data[8];
        uint16_t length = 0;
        if (ble_hs_mbuf_to_flat(ctxt->om, data, sizeof data, &length) != 0 && id != ATTR_CONTROL_POINT) {
            return BLE_ATT_ERR_INVALID_ATTR_VALUE_LEN;
        }
        switch (id) {
        case ATTR_PROTOCOL_MODE:
            if (length < 1) return BLE_ATT_ERR_INVALID_ATTR_VALUE_LEN;
            protocol_mode = data[0];
            send_event(DEBONAIR_EVENT_PROTOCOL_MODE, protocol_mode, 0, NULL, NULL, 0);
            return 0;
        case ATTR_LEDS:
        case ATTR_BOOT_KEYBOARD_OUTPUT:
            if (length > 0) leds = data[0];
            send_event(DEBONAIR_EVENT_OUTPUT, 0, id == ATTR_LEDS ? 1 : 0, NULL, data, length);
            return 0;
        case ATTR_FEATURE:
            if (length > 0) {
                feature = data[0];
                save_feature();
            }
            send_event(DEBONAIR_EVENT_FEATURE, 0, 2, NULL, data, length);
            return 0;
        default:
            // Input reports and the control point accept and ignore writes.
            return 0;
        }
    }
    (void)handle;
    return BLE_ATT_ERR_UNLIKELY;
}

static int access_reference(uint16_t handle, uint16_t attribute, struct ble_gatt_access_ctxt *ctxt, void *arg) {
    (void)handle;
    (void)attribute;
    if (ctxt->op != BLE_GATT_ACCESS_OP_READ_DSC) return BLE_ATT_ERR_UNLIKELY;
    return append(ctxt, arg, 2);
}

#define ATTR(id) ((void *)(intptr_t)(id))
#define READ_ENCRYPTED (BLE_GATT_CHR_F_READ | BLE_GATT_CHR_F_READ_ENC)
#define WRITE_ENCRYPTED (BLE_GATT_CHR_F_WRITE | BLE_GATT_CHR_F_WRITE_ENC)

// NimBLE keeps pointers to these UUIDs for as long as the database exists.
static const ble_uuid16_t UUID_REPORT_REFERENCE = BLE_UUID16_INIT(0x2908);
static const ble_uuid16_t UUID_REPORT = BLE_UUID16_INIT(0x2a4d);
static const ble_uuid16_t UUID_PROTOCOL_MODE = BLE_UUID16_INIT(0x2a4e);
static const ble_uuid16_t UUID_REPORT_MAP = BLE_UUID16_INIT(0x2a4b);
static const ble_uuid16_t UUID_INFORMATION = BLE_UUID16_INIT(0x2a4a);
static const ble_uuid16_t UUID_CONTROL_POINT = BLE_UUID16_INIT(0x2a4c);
static const ble_uuid16_t UUID_BOOT_KEYBOARD = BLE_UUID16_INIT(0x2a22);
static const ble_uuid16_t UUID_BOOT_KEYBOARD_OUTPUT = BLE_UUID16_INIT(0x2a32);
static const ble_uuid16_t UUID_BOOT_MOUSE = BLE_UUID16_INIT(0x2a33);
static const ble_uuid16_t UUID_MANUFACTURER = BLE_UUID16_INIT(0x2a29);
static const ble_uuid16_t UUID_MODEL = BLE_UUID16_INIT(0x2a24);
static const ble_uuid16_t UUID_FIRMWARE = BLE_UUID16_INIT(0x2a26);
static const ble_uuid16_t UUID_PNP = BLE_UUID16_INIT(0x2a50);
static const ble_uuid16_t UUID_BATTERY_LEVEL = BLE_UUID16_INIT(0x2a19);
static const ble_uuid16_t UUID_DEVICE_INFORMATION = BLE_UUID16_INIT(0x180a);
static const ble_uuid16_t UUID_BATTERY = BLE_UUID16_INIT(0x180f);
static const ble_uuid16_t UUID_HID = BLE_UUID16_INIT(0x1812);
static const ble_uuid16_t UUID_GATT = BLE_UUID16_INIT(0x1801);
static const ble_uuid16_t UUID_SERVICE_CHANGED = BLE_UUID16_INIT(0x2a05);

static const uint8_t KEYBOARD_REFERENCE[] = {1, 1};
static const uint8_t LEDS_REFERENCE[] = {1, 2};
static const uint8_t CONSUMER_REFERENCE[] = {3, 1};
static const uint8_t MOUSE_REFERENCE[] = {2, 1};
static const uint8_t FEATURE_REFERENCE[] = {2, 3};

static struct ble_gatt_dsc_def references[5][2];
static struct ble_gatt_chr_def hid[14];
static struct ble_gatt_chr_def device_information[] = {
    {.uuid = &UUID_MANUFACTURER.u, .access_cb = access_attribute, .arg = ATTR(ATTR_MANUFACTURER), .flags = BLE_GATT_CHR_F_READ},
    {.uuid = &UUID_MODEL.u, .access_cb = access_attribute, .arg = ATTR(ATTR_MODEL), .flags = BLE_GATT_CHR_F_READ},
    {.uuid = &UUID_FIRMWARE.u, .access_cb = access_attribute, .arg = ATTR(ATTR_FIRMWARE), .flags = BLE_GATT_CHR_F_READ},
    {.uuid = &UUID_PNP.u, .access_cb = access_attribute, .arg = ATTR(ATTR_PNP), .flags = BLE_GATT_CHR_F_READ},
    {0},
};
static struct ble_gatt_chr_def battery_service[] = {
    {.uuid = &UUID_BATTERY_LEVEL.u, .access_cb = access_attribute, .arg = ATTR(ATTR_BATTERY),
     .flags = BLE_GATT_CHR_F_READ | BLE_GATT_CHR_F_NOTIFY, .val_handle = &battery_handle},
    {0},
};
static struct ble_gatt_svc_def services[] = {
    {.type = BLE_GATT_SVC_TYPE_PRIMARY, .uuid = &UUID_DEVICE_INFORMATION.u, .characteristics = device_information},
    {.type = BLE_GATT_SVC_TYPE_PRIMARY, .uuid = &UUID_BATTERY.u, .characteristics = battery_service},
    {.type = BLE_GATT_SVC_TYPE_PRIMARY, .uuid = &UUID_HID.u, .characteristics = hid},
    {0},
};

static void add(int *at, const ble_uuid_t *uuid, int id, ble_gatt_chr_flags flags, uint16_t *handle) {
    hid[*at] = (struct ble_gatt_chr_def){.uuid = uuid, .access_cb = access_attribute, .arg = ATTR(id), .flags = flags, .val_handle = handle};
    (*at)++;
}

static void add_report(int *at, int *reference, int id, ble_gatt_chr_flags flags, const uint8_t *value, uint16_t *handle) {
    references[*reference][0] = (struct ble_gatt_dsc_def){
        .uuid = &UUID_REPORT_REFERENCE.u, .att_flags = BLE_ATT_F_READ, .access_cb = access_reference, .arg = (void *)value};
    hid[*at] = (struct ble_gatt_chr_def){.uuid = &UUID_REPORT.u, .access_cb = access_attribute, .arg = ATTR(id),
        .flags = flags, .val_handle = handle, .descriptors = references[*reference]};
    (*at)++;
    (*reference)++;
}

static void build_hid(uint8_t devices) {
    memset(hid, 0, sizeof hid);
    memset(references, 0, sizeof references);
    int at = 0;
    int reference = 0;
    add(&at, &UUID_PROTOCOL_MODE.u, ATTR_PROTOCOL_MODE, BLE_GATT_CHR_F_READ | BLE_GATT_CHR_F_WRITE_NO_RSP, NULL);
    if (devices & DEBONAIR_KEYBOARD) {
        add_report(&at, &reference, ATTR_KEYBOARD, READ_ENCRYPTED | BLE_GATT_CHR_F_NOTIFY, KEYBOARD_REFERENCE, &keyboard_handle);
        add_report(&at, &reference, ATTR_LEDS, READ_ENCRYPTED | WRITE_ENCRYPTED | BLE_GATT_CHR_F_WRITE_NO_RSP, LEDS_REFERENCE, NULL);
        add_report(&at, &reference, ATTR_CONSUMER, READ_ENCRYPTED | BLE_GATT_CHR_F_NOTIFY, CONSUMER_REFERENCE, &consumer_handle);
    }
    if (devices & DEBONAIR_MOUSE) {
        add_report(&at, &reference, ATTR_MOUSE, READ_ENCRYPTED | BLE_GATT_CHR_F_NOTIFY, MOUSE_REFERENCE, &mouse_handle);
        add_report(&at, &reference, ATTR_FEATURE, READ_ENCRYPTED | WRITE_ENCRYPTED, FEATURE_REFERENCE, NULL);
    }
    add(&at, &UUID_REPORT_MAP.u, ATTR_REPORT_MAP, BLE_GATT_CHR_F_READ, NULL);
    add(&at, &UUID_INFORMATION.u, ATTR_INFORMATION, BLE_GATT_CHR_F_READ, NULL);
    add(&at, &UUID_CONTROL_POINT.u, ATTR_CONTROL_POINT, BLE_GATT_CHR_F_WRITE_NO_RSP, NULL);
    if (devices & DEBONAIR_KEYBOARD) {
        add(&at, &UUID_BOOT_KEYBOARD.u, ATTR_BOOT_KEYBOARD, READ_ENCRYPTED | BLE_GATT_CHR_F_NOTIFY, &boot_keyboard_handle);
        add(&at, &UUID_BOOT_KEYBOARD_OUTPUT.u, ATTR_BOOT_KEYBOARD_OUTPUT,
            READ_ENCRYPTED | WRITE_ENCRYPTED | BLE_GATT_CHR_F_WRITE_NO_RSP, NULL);
    }
    if (devices & DEBONAIR_MOUSE) {
        add(&at, &UUID_BOOT_MOUSE.u, ATTR_BOOT_MOUSE, READ_ENCRYPTED | BLE_GATT_CHR_F_NOTIFY, &boot_mouse_handle);
    }
}

// ---- Advertising and connections ---------------------------------------------------------------

static int gap_event(struct ble_gap_event *event, void *arg);

static void advertise(void) {
    if (connection != BLE_HS_CONN_HANDLE_NONE) return;
    if (ble_gap_adv_active()) ble_gap_adv_stop();
    struct ble_hs_adv_fields fields = {0};
    // Discoverable only while pairing. BR/EDR is not supported.
    fields.flags = BLE_HS_ADV_F_BREDR_UNSUP | (pairing ? BLE_HS_ADV_F_DISC_GEN : 0);
    fields.appearance = appearance;
    fields.appearance_is_present = 1;
    static ble_uuid16_t hid_uuid = BLE_UUID16_INIT(0x1812);
    fields.uuids16 = &hid_uuid;
    fields.num_uuids16 = 1;
    fields.uuids16_is_complete = 1;
    // Flags (3), appearance (4) and the service list (4) leave 18 bytes for the name.
    uint8_t shown = name_length;
    if (shown > 18) {
        shown = 18;
        while (shown > 0 && ((uint8_t)name[shown] & 0xc0) == 0x80) shown--;
    }
    fields.name = (uint8_t *)name;
    fields.name_len = shown;
    fields.name_is_complete = shown == name_length;
    ble_gap_adv_set_fields(&fields);
    struct ble_hs_adv_fields response = {0};
    response.name = (uint8_t *)name;
    response.name_len = name_length;
    response.name_is_complete = 1;
    ble_gap_adv_rsp_set_fields(&response);
    struct ble_gap_adv_params params = {0};
    params.conn_mode = BLE_GAP_CONN_MODE_UND;
    params.disc_mode = pairing ? BLE_GAP_DISC_MODE_GEN : BLE_GAP_DISC_MODE_NON;
    // Every 30 ms on all channels.
    params.itvl_min = 0x30;
    params.itvl_max = 0x30;
    ble_gap_adv_start(BLE_OWN_ADDR_RANDOM, NULL, BLE_HS_FOREVER, &params, gap_event, NULL);
}

static void encryption_timeout(struct ble_npl_event *event) {
    (void)event;
    if (connection != BLE_HS_CONN_HANDLE_NONE && !encrypted) {
        ble_gap_terminate(connection, BLE_ERR_REM_USER_CONN_TERM);
    }
}

static void refuse(uint16_t handle) {
    // Outside pairing, a Target that cannot pair would otherwise hold the only connection.
    ble_gap_terminate(handle, BLE_ERR_AUTH_FAIL);
}

static void pump(struct ble_npl_event *event);

static int gap_event(struct ble_gap_event *event, void *arg) {
    (void)arg;
    struct ble_gap_conn_desc desc;
    switch (event->type) {
    case BLE_GAP_EVENT_CONNECT:
        if (event->connect.status != 0) {
            advertise();
            break;
        }
        connection = event->connect.conn_handle;
        encrypted = false;
        wrote_keys = false;
        refused = false;
        refused_known = false;
        leds = 0;
        feature = 0;
        protocol_mode = 1;
        memset(last_keyboard, 0, sizeof last_keyboard);
        memset(last_mouse, 0, sizeof last_mouse);
        memset(last_consumer, 0, sizeof last_consumer);
        ble_npl_callout_reset(&encryption_timer,
                              ble_npl_time_ms_to_ticks32(pairing ? UNENCRYPTED_PAIRING_MS : UNENCRYPTED_MS));
        if (ble_gap_conn_find(connection, &desc) == 0) {
            send_event(DEBONAIR_EVENT_CONNECTED, 0, 0, &desc.peer_id_addr, NULL, 0);
        }
        break;
    case BLE_GAP_EVENT_DISCONNECT:
        if (event->disconnect.conn.conn_handle != connection) break;
        connection = BLE_HS_CONN_HANDLE_NONE;
        encrypted = false;
        ble_npl_callout_stop(&encryption_timer);
        xQueueReset(reports);
        // A pairing that did not complete is not kept.
        bool emptied = false;
        portENTER_CRITICAL(&bonds_lock);
        for (int i = 0; i < BONDS; i++) {
            if (records[i].used && !complete(&records[i])) {
                memset(&records[i], 0, sizeof records[i]);
                changed_slots |= 1u << i;
                emptied = true;
            }
        }
        portEXIT_CRITICAL(&bonds_lock);
        if (emptied) simple_event(DEBONAIR_EVENT_BONDS_CHANGED);
        if (refused && refused_known) {
            // A refused pairing still leaves address, subscription and feature records. Never
            // delete without a known identity: an empty address would match every pairing.
            bool paired;
            portENTER_CRITICAL(&bonds_lock);
            paired = find(&refused_peer) >= 0;
            portEXIT_CRITICAL(&bonds_lock);
            if (!paired) ble_store_util_delete_peer(&refused_peer);
        }
        refused = false;
        refused_known = false;
        simple_event(DEBONAIR_EVENT_DISCONNECTED);
        advertise();
        break;
    case BLE_GAP_EVENT_ADV_COMPLETE:
        advertise();
        break;
    case BLE_GAP_EVENT_ENC_CHANGE:
        if (event->enc_change.conn_handle != connection || event->enc_change.status != 0) break;
        if (ble_gap_conn_find(connection, &desc) != 0 || !desc.sec_state.encrypted) break;
        // NimBLE saves a new pairing's keys before this event, and reports the Target's identity
        // address here. A pairing this mode refused gets no input, nor does an encryption without
        // a pairing: HID over GATT requires bonding.
        int slot;
        portENTER_CRITICAL(&bonds_lock);
        slot = find_paired(&desc.peer_id_addr);
        portEXIT_CRITICAL(&bonds_lock);
        if (refused || slot < 0) {
            refused = true;
            refused_peer = desc.peer_id_addr;
            refused_known = true;
            refuse(connection);
            if (pairing) send_event(DEBONAIR_EVENT_PAIRING_COMPLETE, 0, 0, NULL, NULL, 0);
            break;
        }
        if (wrote_keys) {
            wrote_keys = false;
            // The application reports the pairing as made once it is saved.
            debonair_event paired = {.kind = DEBONAIR_EVENT_PAIRING_COMPLETE,
                                     .code = 1,
                                     .entry = (int8_t)slot,
                                     .random = desc.peer_id_addr.type & 1};
            reverse(paired.address, desc.peer_id_addr.val, 6);
            emit(&paired);
            simple_event(DEBONAIR_EVENT_TARGETS_CHANGED);
        }
        encrypted = true;
        ble_npl_callout_stop(&encryption_timer);
        simple_event(DEBONAIR_EVENT_SECURED);
        restore_bond(&desc.peer_id_addr);
        // 15 ms, within Apple's accessory rules, with a 3 s supervision timeout.
        struct ble_gap_upd_params params = {
            .itvl_min = 12, .itvl_max = 12, .latency = 0, .supervision_timeout = 300};
        ble_gap_update_params(connection, &params);
        break;
    case BLE_GAP_EVENT_PARING_COMPLETE:
        // Also reported when a paired Target re-encrypts; success is reported with encryption.
        if (event->pairing_complete.conn_handle == connection && event->pairing_complete.status != 0) {
            send_event(DEBONAIR_EVENT_PAIRING_COMPLETE, 0, 0, NULL, NULL, 0);
        }
        break;
    case BLE_GAP_EVENT_PASSKEY_ACTION: {
        uint16_t handle = event->passkey.conn_handle;
        if (event->passkey.params.action == BLE_SM_IOACT_INPUT && pairing && handle == connection) {
            simple_event(DEBONAIR_EVENT_PASSKEY_REQUEST);
        } else if (event->passkey.params.action == BLE_SM_IOACT_NUMCMP) {
            // The adapter shows nothing to compare, so this method is never offered; refuse it.
            struct ble_sm_io io = {.action = BLE_SM_IOACT_NUMCMP, .numcmp_accept = 0};
            ble_sm_inject_io(handle, &io);
        } else if (event->passkey.params.action != BLE_SM_IOACT_NONE) {
            refuse(handle);
        }
        break;
    }
    case BLE_GAP_EVENT_REPEAT_PAIRING:
        // A paired Target that lost its keys pairs again only while pairing, and in passkey mode only
        // with the passkey; otherwise its existing pairing stays.
        if (!pairing || (passkey_mode && !event->repeat_pairing.new_authenticated)) {
            return BLE_GAP_REPEAT_PAIRING_IGNORE;
        }
        if (ble_gap_conn_find(event->repeat_pairing.conn_handle, &desc) == 0) {
            // The new pairing adds the Target's new identity key to the resolving list.
            ble_hs_pvcy_remove_entry(desc.peer_id_addr.type, desc.peer_id_addr.val);
            delete_for_pairing(&desc.peer_id_addr);
        }
        return BLE_GAP_REPEAT_PAIRING_RETRY;
    case BLE_GAP_EVENT_IDENTITY_RESOLVED: {
        // Reported while a pairing saves its keys, before they are written.
        if (event->identity_resolved.conn_handle != connection || !pairing) break;
        const ble_addr_t *identity_address = &event->identity_resolved.peer_id_addr;
        // NimBLE sets the link's authentication before keys are exchanged. A pairing passkey mode
        // will refuse must not remove the existing one.
        if (passkey_mode && (ble_gap_conn_find(connection, &desc) != 0 || !desc.sec_state.authenticated)) break;
        bool paired;
        portENTER_CRITICAL(&bonds_lock);
        paired = find(identity_address) >= 0;
        portEXIT_CRITICAL(&bonds_lock);
        if (paired) {
            // A paired Target pairing again with a new identity key: the controller accepts the new
            // key only once the old one is gone.
            ble_hs_pvcy_remove_entry(identity_address->type, identity_address->val);
            delete_for_pairing(identity_address);
        }
        send_event(DEBONAIR_EVENT_IDENTIFIED, 0, 0, identity_address, NULL, 0);
        break;
    }
    case BLE_GAP_EVENT_NOTIFY_TX:
        // The Target confirmed Service Changed, so it rediscovers this database.
        if (event->notify_tx.indication && event->notify_tx.attr_handle == service_changed_handle &&
            event->notify_tx.status == BLE_HS_EDONE) {
            save_layout();
        }
        break;
    default:
        break;
    }
    return 0;
}

// ---- Reports and commands ------------------------------------------------------------------

static uint16_t report_handle(const report *r) {
    if (r->channel == DEBONAIR_CHANNEL_BOOT_KEYBOARD) return boot_keyboard_handle;
    if (r->channel == DEBONAIR_CHANNEL_BOOT_MOUSE) return boot_mouse_handle;
    switch (r->report_id) {
    case 1: return keyboard_handle;
    case 2: return mouse_handle;
    case 3: return consumer_handle;
    default: return 0;
    }
}

static void remember(const report *r) {
    uint8_t *last = NULL;
    size_t size = 0;
    if (r->channel == DEBONAIR_CHANNEL_BOOT_KEYBOARD || (r->channel == DEBONAIR_CHANNEL_INPUT && r->report_id == 1)) {
        last = last_keyboard;
        size = sizeof last_keyboard;
    } else if (r->channel == DEBONAIR_CHANNEL_INPUT && r->report_id == 3) {
        last = last_consumer;
        size = sizeof last_consumer;
    } else {
        // A read of the mouse report returns held buttons only; motion is relative.
        memset(last_mouse, 0, sizeof last_mouse);
        if (r->length > 0) last_mouse[0] = r->data[0];
        return;
    }
    memset(last, 0, size);
    memcpy(last, r->data, r->length < size ? r->length : size);
}

/** Free buffers in NimBLE's first pool, which notifications use and never fall back from. Blocks
 * of the second pool in use, for long attribute values, briefly count against it too, which only
 * delays reports. */
static int pool_free(void) {
    return os_msys_num_free() - MYNEWT_VAL(MSYS_2_BLOCK_COUNT);
}

/** Sends queued reports in order until NimBLE runs out of buffers, then waits for one. */
static void pump(struct ble_npl_event *event) {
    (void)event;
    report r;
    while (xQueuePeek(reports, &r, 0) == pdTRUE) {
        uint16_t handle = report_handle(&r);
        if (connection == BLE_HS_CONN_HANDLE_NONE || !encrypted || handle == 0) {
            xQueueReceive(reports, &r, 0);
            continue;
        }
        struct os_mbuf *om = pool_free() >= FREE_BUFFERS ? ble_hs_mbuf_from_flat(r.data, r.length) : NULL;
        if (!om) {
            ble_npl_callout_reset(&retry_timer, ble_npl_time_ms_to_ticks32(RETRY_MS));
            return;
        }
        // NimBLE consumes the buffer whatever the result.
        int status = ble_gatts_notify_custom(connection, handle, om);
        if (status == BLE_HS_ENOMEM || status == BLE_HS_EBUSY) {
            ble_npl_callout_reset(&retry_timer, ble_npl_time_ms_to_ticks32(RETRY_MS));
            return;
        }
        xQueueReceive(reports, &r, 0);
        if (status == 0) remember(&r);
    }
    if (want_send) {
        want_send = false;
        simple_event(DEBONAIR_EVENT_CAN_SEND);
    }
}

static void retry(struct ble_npl_event *event) { pump(event); }

static void set_security(bool passkey) {
    passkey_mode = passkey;
    ble_hs_cfg.sm_io_cap = passkey ? BLE_HS_IO_KEYBOARD_ONLY : BLE_HS_IO_NO_INPUT_OUTPUT;
    // MITM protection makes the Target enter the passkey rather than fall back to Just Works.
    ble_hs_cfg.sm_mitm = passkey ? 1 : 0;
}

static void command(const debonair_command *c) {
    switch (c->kind) {
    case DEBONAIR_COMMAND_NAME:
        memcpy(name, c->name, c->name_length);
        name[c->name_length] = 0;
        name_length = c->name_length;
        ble_svc_gap_device_name_set(name);
        advertise();
        break;
    case DEBONAIR_COMMAND_SECURITY:
        set_security(c->value != 0);
        break;
    case DEBONAIR_COMMAND_PAIRING:
        if (pairing != (c->value != 0)) {
            pairing = c->value != 0;
            advertise();
        }
        break;
    case DEBONAIR_COMMAND_PASSKEY:
        if (connection == BLE_HS_CONN_HANDLE_NONE) break;
        if (c->value) {
            struct ble_sm_io io = {.action = BLE_SM_IOACT_INPUT, .passkey = c->passkey};
            ble_sm_inject_io(connection, &io);
        } else {
            refuse(connection);
        }
        break;
    case DEBONAIR_COMMAND_DISCONNECT:
        if (connection != BLE_HS_CONN_HANDLE_NONE) ble_gap_terminate(connection, BLE_ERR_REM_USER_CONN_TERM);
        break;
    case DEBONAIR_COMMAND_FORGET: {
        ble_addr_t address = {.type = c->value & 1};
        for (int i = 0; i < 6; i++) address.val[i] = c->address[5 - i];
        // The controller changes its resolving list only while not advertising. Unpairing
        // disconnects the Target if it is connected and removes its keys, subscriptions and
        // resolving list entry.
        if (ble_gap_adv_active()) ble_gap_adv_stop();
        if (ble_gap_unpair(&address) != 0) ble_store_util_delete_peer(&address);
        advertise();
        __atomic_store_n(&forgotten, c->sequence, __ATOMIC_RELEASE);
        simple_event(DEBONAIR_EVENT_TARGETS_CHANGED);
        break;
    }
    case DEBONAIR_COMMAND_BATTERY:
        battery = c->value;
        ble_gatts_chr_updated(battery_handle);
        break;
    default:
        break;
    }
}

static void process_commands(struct ble_npl_event *event) {
    (void)event;
    debonair_command c;
    while (xQueueReceive(commands, &c, 0) == pdTRUE) command(&c);
}

int debonair_nimble_command(const debonair_command *c) {
    if (!commands || !c || c->name_length > NAME_BYTES) return -1;
    if (xQueueSend(commands, c, 0) != pdTRUE) return -1;
    ble_npl_eventq_put(nimble_port_get_dflt_eventq(), &command_event);
    return 0;
}

uint32_t debonair_nimble_forgotten(void) { return __atomic_load_n(&forgotten, __ATOMIC_ACQUIRE); }

int debonair_nimble_can_send(void) {
    // A short queue keeps latency low; the application merges motion while it waits.
    if (connection != BLE_HS_CONN_HANDLE_NONE && encrypted && reports && uxQueueMessagesWaiting(reports) < 4) return 1;
    want_send = true;
    return 0;
}

int debonair_nimble_send(uint8_t channel, uint8_t report_id, const uint8_t *data, uint16_t length) {
    if (connection == BLE_HS_CONN_HANDLE_NONE || !encrypted || length > sizeof(((report *)0)->data)) return -1;
    report r = {.channel = channel, .report_id = report_id, .length = length};
    memcpy(r.data, data, length);
    if (xQueueSend(reports, &r, 0) != pdTRUE) return -1;
    ble_npl_eventq_put(nimble_port_get_dflt_eventq(), &report_event);
    return 0;
}

// ---- Startup ---------------------------------------------------------------------------------

static void synced(void) {
    // NimBLE synchronizes again after a host reset; the application is told once.
    // Each device combination has its own identity, so Targets pair with it as a separate device.
    if (ble_hs_id_set_rnd(identity) != 0) {
        send_event(DEBONAIR_EVENT_FAILED, 0, 0, NULL, NULL, 0);
        return;
    }
    ble_gatts_find_chr(&UUID_GATT.u, &UUID_SERVICE_CHANGED.u, NULL, &service_changed_handle);
    advertise();
    if (!ready_sent) {
        ready_sent = true;
        simple_event(DEBONAIR_EVENT_READY);
    }
}

static void reset(int reason) {
    (void)reason;
    // NimBLE restarts its host after a controller error and synchronizes again.
    ble_npl_callout_stop(&encryption_timer);
    ble_npl_callout_stop(&retry_timer);
    xQueueReset(reports);
    if (connection != BLE_HS_CONN_HANDLE_NONE) {
        connection = BLE_HS_CONN_HANDLE_NONE;
        encrypted = false;
        simple_event(DEBONAIR_EVENT_DISCONNECTED);
    }
}

static void host_task(void *arg) {
    (void)arg;
    nimble_port_run();
    nimble_port_freertos_deinit();
}

static void copy_text(char *to, size_t capacity, const char *from) {
    size_t length = strlen(from);
    if (length >= capacity) length = capacity - 1;
    memcpy(to, from, length);
    to[length] = 0;
}

int debonair_nimble_start(const debonair_hid_config *config, const uint8_t irk[16], debonair_emit callback) {
    if (!(config->devices & (DEBONAIR_KEYBOARD | DEBONAIR_MOUSE))) return -1;
    if (config->descriptor_length > DESCRIPTOR_BYTES) return -1;
    if (config->name_length == 0 || config->name_length > NAME_BYTES) return -1;
    emit = callback;
    memcpy(name, config->name, config->name_length);
    name[config->name_length] = 0;
    name_length = config->name_length;
    memcpy(descriptor, config->descriptor, config->descriptor_length);
    descriptor_length = config->descriptor_length;
    copy_text(manufacturer, sizeof manufacturer, config->manufacturer);
    copy_text(model, sizeof model, config->model);
    copy_text(firmware, sizeof firmware, config->firmware);
    // Vendor ID source 2: a USB-IF vendor ID.
    pnp[0] = 2;
    pnp[1] = config->vendor_id;
    pnp[2] = config->vendor_id >> 8;
    pnp[3] = config->product_id;
    pnp[4] = config->product_id >> 8;
    pnp[5] = 0x00;
    pnp[6] = 0x01;
    for (int i = 0; i < 6; i++) identity[i] = config->identity[5 - i];
    appearance = config->devices == DEBONAIR_KEYBOARD ? 0x03c1 : config->devices == DEBONAIR_MOUSE ? 0x03c2 : 0x03c0;
    battery = config->battery;
    // FNV-1a of the report map and device combination, which decide the database layout.
    layout = 2166136261u;
    for (uint16_t i = 0; i < descriptor_length; i++) layout = (layout ^ descriptor[i]) * 16777619u;
    layout = (layout ^ config->devices) * 16777619u;
    if (layout == 0) layout = 1;
    reverse(local_irk, irk, 16);

    commands = xQueueCreate(8, sizeof(debonair_command));
    reports = xQueueCreate(16, sizeof(report));
    if (!commands || !reports) return -1;
    if (nimble_port_init() != 0) return -1;

    ble_hs_cfg.sync_cb = synced;
    ble_hs_cfg.reset_cb = reset;
    ble_hs_cfg.store_status_cb = store_status;
    ble_hs_cfg.sm_bonding = 1;
    ble_hs_cfg.sm_sc = 1;
    ble_hs_cfg.sm_our_key_dist = BLE_SM_PAIR_KEY_DIST_ENC | BLE_SM_PAIR_KEY_DIST_ID;
    ble_hs_cfg.sm_their_key_dist = BLE_SM_PAIR_KEY_DIST_ENC | BLE_SM_PAIR_KEY_DIST_ID;
    set_security(config->passkey != 0);
    // Subscriptions stay in NimBLE's RAM store; security keys use the records above.
    ble_store_config_init();
    ble_hs_cfg.store_read_cb = store_read;
    ble_hs_cfg.store_write_cb = store_write;
    ble_hs_cfg.store_delete_cb = store_delete;
    ble_hs_cfg.store_gen_key_cb = generate_key;

    ble_svc_gap_init();
    ble_svc_gatt_init();
    ble_svc_gap_device_name_set(name);
    ble_svc_gap_device_appearance_set(appearance);
    build_hid(config->devices);
    if (ble_gatts_count_cfg(services) != 0 || ble_gatts_add_svcs(services) != 0) return -1;

    ble_npl_event_init(&command_event, process_commands, NULL);
    ble_npl_event_init(&report_event, pump, NULL);
    ble_npl_callout_init(&encryption_timer, nimble_port_get_dflt_eventq(), encryption_timeout, NULL);
    ble_npl_callout_init(&retry_timer, nimble_port_get_dflt_eventq(), retry, NULL);
    nimble_port_freertos_init(host_task);
    return 0;
}
