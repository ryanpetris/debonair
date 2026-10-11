// BTstack's LE device database and TLV over a RAM table of bonds. Every function runs on the Rust
// owner task, which also runs the BTstack run loop.
#include "bond_db.h"
#include <string.h>
#include "btstack_config.h"
#include "btstack_util.h"
#include "ble/le_device_db.h"

_Static_assert(sizeof(debonair_keys) == 64, "debonair_keys layout");
_Static_assert(sizeof(debonair_bond) == 188, "debonair_bond layout");

typedef struct {
    bool used;
    // Higher is a newer pairing. A pairing beyond the table replaces the oldest.
    uint32_t order;
    debonair_bond bond;
} entry;

static entry entries[DEBONAIR_BONDS];
static uint32_t newest;
static uint32_t changed_slots, added_slots, updated_slots;
static uint8_t identity_root[16], encryption_root[16];
static bool have_identity_root, have_encryption_root;
// Both roots were made by the security manager and not yet taken.
static bool roots_made;

static bool valid(int slot) { return slot >= 0 && slot < DEBONAIR_BONDS && entries[slot].used; }

void debonair_bonds_reset(const uint8_t ir[16], const uint8_t er[16]) {
    memset(entries, 0, sizeof entries);
    newest = 0;
    changed_slots = added_slots = updated_slots = 0;
    have_identity_root = ir != NULL;
    have_encryption_root = er != NULL;
    roots_made = false;
    memset(identity_root, 0, 16);
    memset(encryption_root, 0, 16);
    if (ir) memcpy(identity_root, ir, 16);
    if (er) memcpy(encryption_root, er, 16);
}

int debonair_bonds_take_roots(uint8_t ir[16], uint8_t er[16]) {
    if (!roots_made) return 0;
    roots_made = false;
    memcpy(ir, identity_root, 16);
    memcpy(er, encryption_root, 16);
    return 1;
}

int debonair_bonds_import(const debonair_bond *bond) {
    for (int i = 0; i < DEBONAIR_BONDS; i++) {
        if (entries[i].used) continue;
        entries[i].used = true;
        entries[i].order = ++newest;
        entries[i].bond = *bond;
        return i;
    }
    return -1;
}

int debonair_bonds_export(int slot, debonair_bond *out) {
    if (!valid(slot)) return 0;
    *out = entries[slot].bond;
    return 1;
}

uint32_t debonair_bonds_take(uint32_t *added, uint32_t *updated) {
    uint32_t changed = changed_slots;
    *added = added_slots;
    *updated = updated_slots;
    changed_slots = added_slots = updated_slots = 0;
    return changed;
}

void debonair_bonds_paired(int slot) {
    if (!valid(slot)) return;
    entries[slot].order = ++newest;
    added_slots |= 1u << slot;
}

void debonair_bonds_set_feature(int slot, uint8_t value) {
    if (!valid(slot)) return;
    debonair_bond *b = &entries[slot].bond;
    if ((b->flags & DEBONAIR_BOND_FEATURE) && b->feature == value) return;
    b->flags |= DEBONAIR_BOND_FEATURE;
    b->feature = value;
    updated_slots |= 1u << slot;
}

bool debonair_bonds_feature(int slot, uint8_t *value) {
    if (!valid(slot) || !(entries[slot].bond.flags & DEBONAIR_BOND_FEATURE)) return false;
    *value = entries[slot].bond.feature;
    return true;
}

void debonair_bonds_set_database(int slot, const uint8_t *id, uint8_t length) {
    if (!valid(slot) || length > DEBONAIR_DATABASE_BYTES || debonair_bonds_database_is(slot, id, length)) return;
    debonair_bond *b = &entries[slot].bond;
    memset(b->database, 0, sizeof b->database);
    memcpy(b->database, id, length);
    b->database_length = length;
    updated_slots |= 1u << slot;
}

bool debonair_bonds_database_is(int slot, const uint8_t *id, uint8_t length) {
    if (!valid(slot)) return false;
    const debonair_bond *b = &entries[slot].bond;
    return b->database_length == length && (length == 0 || memcmp(b->database, id, length) == 0);
}

void debonair_bonds_keep_subscriptions(const uint8_t *id, uint8_t length) {
    for (int i = 0; i < DEBONAIR_BONDS; i++) {
        if (entries[i].used && !debonair_bonds_database_is(i, id, length)) {
            memset(entries[i].bond.subscriptions, 0, sizeof entries[i].bond.subscriptions);
        }
    }
}

// ---- LE device database -------------------------------------------------------------------

void le_device_db_init(void) {}

void le_device_db_set_local_bd_addr(bd_addr_t addr) { (void)addr; }

int le_device_db_count(void) {
    int count = 0;
    for (int i = 0; i < DEBONAIR_BONDS; i++) count += entries[i].used;
    return count;
}

int le_device_db_max_count(void) { return DEBONAIR_BONDS; }

int le_device_db_add(int addr_type, bd_addr_t addr, sm_key_t irk) {
    int slot = -1;
    int empty = -1;
    int oldest = -1;
    for (int i = 0; i < DEBONAIR_BONDS; i++) {
        const entry *e = &entries[i];
        if (!e->used) {
            if (empty < 0) empty = i;
            continue;
        }
        if (e->bond.random == (addr_type & 1) && memcmp(e->bond.address, addr, 6) == 0) slot = i;
        if (oldest < 0 || e->order < entries[oldest].order) oldest = i;
    }
    if (slot < 0) slot = empty >= 0 ? empty : oldest;
    if (slot < 0) return -1;
    entry *e = &entries[slot];
    memset(e, 0, sizeof *e);
    e->used = true;
    e->order = ++newest;
    memcpy(e->bond.address, addr, 6);
    e->bond.random = addr_type & 1;
    e->bond.peer.flags = DEBONAIR_KEYS_PRESENT;
    static const sm_key_t zero = {0};
    if (irk && memcmp(irk, zero, 16) != 0) {
        e->bond.peer.flags |= DEBONAIR_KEY_IRK;
        memcpy(e->bond.peer.irk, irk, 16);
    }
    added_slots |= 1u << slot;
    changed_slots |= 1u << slot;
    return slot;
}

void le_device_db_info(int index, int *addr_type, bd_addr_t addr, sm_key_t irk) {
    if (!valid(index)) {
        if (addr_type) *addr_type = BD_ADDR_TYPE_UNKNOWN;
        if (addr) memset(addr, 0, 6);
        if (irk) memset(irk, 0, 16);
        return;
    }
    const debonair_bond *b = &entries[index].bond;
    if (addr_type) *addr_type = b->random ? BD_ADDR_TYPE_LE_RANDOM : BD_ADDR_TYPE_LE_PUBLIC;
    if (addr) memcpy(addr, b->address, 6);
    if (irk) memcpy(irk, b->peer.irk, 16);
}

// The security manager sets the keys only when a pairing completes, including when a paired Target
// pairs again in the same entry, so the entry becomes the newest pairing.
void le_device_db_encryption_set(int index, uint16_t ediv, uint8_t rand[8], sm_key_t ltk, int key_size,
                                 int authenticated, int authorized, int secure_connection) {
    if (!valid(index)) return;
    entries[index].order = ++newest;
    added_slots |= 1u << index;
    debonair_bond *b = &entries[index].bond;
    b->peer.flags |= DEBONAIR_KEYS_PRESENT | DEBONAIR_KEY_LTK;
    b->peer.ediv = ediv;
    memcpy(b->peer.rand, rand, 8);
    memcpy(b->peer.ltk, ltk, 16);
    b->key_size = (uint8_t)key_size;
    b->flags = (b->flags & DEBONAIR_BOND_FEATURE) | (authenticated ? DEBONAIR_BOND_AUTHENTICATED : 0) |
               (authorized ? DEBONAIR_BOND_AUTHORIZED : 0) |
               (secure_connection ? DEBONAIR_BOND_SECURE_CONNECTIONS : 0);
    changed_slots |= 1u << index;
}

void le_device_db_encryption_get(int index, uint16_t *ediv, uint8_t rand[8], sm_key_t ltk, int *key_size,
                                 int *authenticated, int *authorized, int *secure_connection) {
    if (!valid(index)) return;
    const debonair_bond *b = &entries[index].bond;
    if (ediv) *ediv = b->peer.ediv;
    if (rand) memcpy(rand, b->peer.rand, 8);
    if (ltk) memcpy(ltk, b->peer.ltk, 16);
    if (key_size) *key_size = b->key_size;
    if (authenticated) *authenticated = (b->flags & DEBONAIR_BOND_AUTHENTICATED) != 0;
    if (authorized) *authorized = (b->flags & DEBONAIR_BOND_AUTHORIZED) != 0;
    if (secure_connection) *secure_connection = (b->flags & DEBONAIR_BOND_SECURE_CONNECTIONS) != 0;
}

void le_device_db_remove(int index) {
    if (!valid(index)) return;
    memset(&entries[index], 0, sizeof entries[index]);
    changed_slots |= 1u << index;
}

void le_device_db_dump(void) {}

// ---- TLV --------------------------------------------------------------------------------------
// BTstack's security manager reads its identity roots and its ATT server reads and writes the
// subscriptions of bonded Targets through the TLV. A combination without saved roots gets them from
// the security manager, which makes each once from the controller's random numbers. Each
// subscription tag maps to one subscription of one bond: tag index / DEBONAIR_SUBSCRIPTIONS is
// the entry. The ATT server's own database hash is not kept: each bond keeps the identifier of the
// database its subscriptions belong to.
// Other tags hold nothing and refuse writes.

#define TAG(a, b, c, d) (((uint32_t)(a) << 24) | ((uint32_t)(b) << 16) | ((uint32_t)(c) << 8) | (uint32_t)(d))
#define TAG_IDENTITY_ROOT TAG('S', 'M', 'I', 'R')
#define TAG_ENCRYPTION_ROOT TAG('S', 'M', 'E', 'R')
#define TAG_SUBSCRIPTION TAG('B', 'T', 'C', 0)
#define TAG_DATABASE_HASH TAG('B', 'T', 'D', 'B')

// The ATT server's persistent_ccc_entry_t.
typedef struct {
    uint32_t seq_nr;
    uint16_t att_handle;
    uint8_t value;
    uint8_t device_index;
} subscription_entry;
_Static_assert(sizeof(subscription_entry) == 8, "ATT server subscription entry layout");
_Static_assert(DEBONAIR_BONDS * DEBONAIR_SUBSCRIPTIONS == NVN_NUM_GATT_SERVER_CCC,
               "a subscription tag for each subscription of each bond");

static debonair_subscription *subscription_for_tag(uint32_t tag, int *slot) {
    if ((tag & 0xffffff00u) != TAG_SUBSCRIPTION) return NULL;
    uint32_t index = tag & 0xff;
    *slot = (int)(index / DEBONAIR_SUBSCRIPTIONS);
    if (!valid(*slot)) return NULL;
    return &entries[*slot].bond.subscriptions[index % DEBONAIR_SUBSCRIPTIONS];
}

static int tlv_get(void *context, uint32_t tag, uint8_t *buffer, uint32_t size) {
    (void)context;
    if (buffer && size) memset(buffer, 0, size);
    const uint8_t *value = NULL;
    uint32_t length = 0;
    subscription_entry subscription;
    int slot;
    debonair_subscription *s;
    if (tag == TAG_IDENTITY_ROOT && have_identity_root) {
        value = identity_root;
        length = 16;
    } else if (tag == TAG_ENCRYPTION_ROOT && have_encryption_root) {
        value = encryption_root;
        length = 16;
    } else if ((s = subscription_for_tag(tag, &slot)) != NULL && s->handle != 0) {
        subscription = (subscription_entry){
            .seq_nr = 1, .att_handle = s->handle, .value = (uint8_t)s->value, .device_index = (uint8_t)slot};
        value = (const uint8_t *)&subscription;
        length = sizeof subscription;
    } else {
        return 0;
    }
    if (buffer) memcpy(buffer, value, btstack_min(size, length));
    return (int)length;
}

static int tlv_store(void *context, uint32_t tag, const uint8_t *data, uint32_t size) {
    (void)context;
    // The ATT server's database hash: the bonds keep their own.
    if (tag == TAG_DATABASE_HASH) return 0;
    // Each root is stored once, when the security manager makes it. Roots are never replaced.
    if ((tag == TAG_IDENTITY_ROOT || tag == TAG_ENCRYPTION_ROOT) && size == 16 && data) {
        bool *have = tag == TAG_IDENTITY_ROOT ? &have_identity_root : &have_encryption_root;
        if (*have) return 1;
        memcpy(tag == TAG_IDENTITY_ROOT ? identity_root : encryption_root, data, 16);
        *have = true;
        roots_made = have_identity_root && have_encryption_root;
        return 0;
    }
    if ((tag & 0xffffff00u) != TAG_SUBSCRIPTION || size != sizeof(subscription_entry) || !data) return 1;
    subscription_entry value;
    memcpy(&value, data, sizeof value);
    if (!valid(value.device_index) || value.att_handle == 0) return 1;
    // The subscription goes with its bond, whichever tag the ATT server chose.
    debonair_subscription *subscriptions = entries[value.device_index].bond.subscriptions;
    debonair_subscription *target = NULL;
    for (int i = 0; i < DEBONAIR_SUBSCRIPTIONS; i++) {
        debonair_subscription *s = &subscriptions[i];
        if (s->handle == value.att_handle) {
            target = s;
            break;
        }
        if (s->handle == 0 && !target) target = s;
    }
    if (!target) return 1;
    if (target->handle != value.att_handle || target->value != value.value) {
        target->handle = value.att_handle;
        target->value = value.value;
        updated_slots |= 1u << value.device_index;
    }
    return 0;
}

static void tlv_delete(void *context, uint32_t tag) {
    (void)context;
    int slot;
    debonair_subscription *s = subscription_for_tag(tag, &slot);
    if (!s || s->handle == 0) return;
    memset(s, 0, sizeof *s);
    updated_slots |= 1u << slot;
}

static const btstack_tlv_t tlv = {
    .get_tag = tlv_get,
    .store_tag = tlv_store,
    .delete_tag = tlv_delete,
};

const btstack_tlv_t *debonair_bonds_tlv(void) { return &tlv; }
