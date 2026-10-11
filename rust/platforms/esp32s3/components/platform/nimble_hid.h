#pragma once
// HID over GATT peripheral on ESP-IDF's NimBLE host. Every NimBLE call runs on the NimBLE host
// task; the application task submits commands and reports through queues and receives events
// through a callback that copies them before returning.
#include <stdbool.h>
#include <stdint.h>

enum {
    DEBONAIR_EVENT_READY = 1,
    DEBONAIR_EVENT_CONNECTED = 2,
    DEBONAIR_EVENT_IDENTIFIED = 3,
    DEBONAIR_EVENT_DISCONNECTED = 4,
    DEBONAIR_EVENT_SECURED = 5,
    DEBONAIR_EVENT_PASSKEY_REQUEST = 6,
    DEBONAIR_EVENT_PAIRING_COMPLETE = 7,
    DEBONAIR_EVENT_OUTPUT = 8,
    DEBONAIR_EVENT_FEATURE = 9,
    DEBONAIR_EVENT_PROTOCOL_MODE = 10,
    DEBONAIR_EVENT_TARGETS_CHANGED = 11,
    DEBONAIR_EVENT_CAN_SEND = 12,
    // The pairing records changed; take the changes and save them.
    DEBONAIR_EVENT_BONDS_CHANGED = 13,
    DEBONAIR_EVENT_FAILED = 14,
};

typedef struct {
    uint8_t kind;
    // Success flag, protocol mode or address type, depending on kind.
    uint8_t code;
    uint8_t report_id;
    // For a successful PAIRING_COMPLETE, the pairing record entry that holds the pairing, or -1 for
    // none, and whether `address`, the Target's identity address, is random.
    int8_t entry;
    uint8_t random;
    // Most significant byte first.
    uint8_t address[6];
    uint16_t length;
    const uint8_t *data;
} debonair_event;

typedef void (*debonair_emit)(const debonair_event *event);

enum {
    DEBONAIR_KEYBOARD = 1,
    DEBONAIR_MOUSE = 2,
};

enum {
    DEBONAIR_CHANNEL_INPUT = 0,
    DEBONAIR_CHANNEL_BOOT_KEYBOARD = 1,
    DEBONAIR_CHANNEL_BOOT_MOUSE = 2,
};

typedef struct {
    // Copied; up to 29 bytes.
    const char *name;
    uint16_t name_length;
    // DEBONAIR_KEYBOARD and/or DEBONAIR_MOUSE.
    uint8_t devices;
    // Nonzero for passkey entry, zero for Just Works.
    uint8_t passkey;
    uint8_t battery;
    // Copied; up to 512 bytes.
    const uint8_t *descriptor;
    uint16_t descriptor_length;
    // Copied; NUL-terminated.
    const char *manufacturer;
    const char *model;
    const char *firmware;
    uint16_t vendor_id;
    uint16_t product_id;
    // Static random identity address, most significant byte first.
    uint8_t identity[6];
} debonair_hid_config;

enum {
    DEBONAIR_COMMAND_NAME = 1,
    DEBONAIR_COMMAND_SECURITY,
    DEBONAIR_COMMAND_PAIRING,
    DEBONAIR_COMMAND_PASSKEY,
    DEBONAIR_COMMAND_DISCONNECT,
    DEBONAIR_COMMAND_FORGET,
    DEBONAIR_COMMAND_BATTERY,
};

typedef struct {
    uint8_t kind;
    // Pairing enabled, passkey security, passkey accepted, battery level or address type.
    uint8_t value;
    uint32_t passkey;
    // Most significant byte first.
    uint8_t address[6];
    uint8_t name_length;
    char name[29];
    // For FORGET: reported by debonair_nimble_forgotten once the pairing is removed.
    uint32_t sequence;
} debonair_command;

#define DEBONAIR_BONDS 8
#define DEBONAIR_SUBSCRIPTIONS 8
#define DEBONAIR_DATABASE_BYTES 16

// debonair_keys.flags
enum {
    DEBONAIR_KEY_LTK = 1,
    DEBONAIR_KEY_IRK = 2,
    DEBONAIR_KEY_CSRK = 4,
    // The side's keys are kept, whichever keys it distributed.
    DEBONAIR_KEYS_PRESENT = 0x80,
};

// Keys one side of a pairing distributed, most significant byte first.
typedef struct {
    uint8_t flags;
    uint16_t ediv;
    uint8_t rand[8];
    uint8_t ltk[16];
    uint8_t irk[16];
    uint8_t csrk[16];
    uint32_t sign_counter;
} debonair_keys;

typedef struct {
    // Zero for none.
    uint16_t handle;
    uint16_t value;
} debonair_subscription;

// debonair_bond.flags
enum {
    DEBONAIR_BOND_AUTHENTICATED = 1,
    DEBONAIR_BOND_AUTHORIZED = 2,
    DEBONAIR_BOND_SECURE_CONNECTIONS = 4,
    // `feature` holds the feature report the Target set.
    DEBONAIR_BOND_FEATURE = 8,
};

// A bond in the form the application saves: what the adapter distributed in `local` and what the
// Target distributed in `peer`. Subscriptions stay in NimBLE's RAM store and are never set here.
typedef struct {
    // The identity address, most significant byte first.
    uint8_t address[6];
    uint8_t random;
    uint8_t key_size;
    uint8_t flags;
    uint8_t feature;
    uint8_t database_length;
    uint8_t database[DEBONAIR_DATABASE_BYTES];
    debonair_keys local;
    debonair_keys peer;
    debonair_subscription subscriptions[DEBONAIR_SUBSCRIPTIONS];
} debonair_bond;

// Empties the pairing records. Call before debonair_nimble_start.
void debonair_nimble_bonds_reset(void);
// Adds a saved bond, oldest first, before debonair_nimble_start. Returns its entry, or -1 when
// the records are full.
int debonair_nimble_bonds_import(const debonair_bond *bond);
// Copies entry `slot`. Returns 0 for an empty entry, 1 for a complete pairing and 2 for an
// incomplete one, whose keys are not all written.
int debonair_nimble_bonds_export(int slot, debonair_bond *out);
// The entries that changed since the last call, as one bit per entry: returns those whose keys
// changed or that were removed, sets `added` to those that hold a new pairing and `updated` to
// those whose feature report or database identifier changed.
uint32_t debonair_nimble_bonds_take(uint32_t *added, uint32_t *updated);
// Starts NimBLE with the imported pairing records. `irk` is the identity resolving key the adapter
// distributes, most significant byte first. Call once.
int debonair_nimble_start(const debonair_hid_config *config, const uint8_t irk[16], debonair_emit emit);
// Queues a command for the host task. Returns nonzero when the queue is full.
int debonair_nimble_command(const debonair_command *command);
// The sequence of the last FORGET command the host task carried out.
uint32_t debonair_nimble_forgotten(void);
// Whether a report can be queued now. When not, a CAN_SEND event follows once it can.
int debonair_nimble_can_send(void);
// Queues an input report. Returns nonzero when it cannot be sent.
int debonair_nimble_send(uint8_t channel, uint8_t report_id, const uint8_t *data, uint16_t length);
// Writes up to capacity paired Targets as 7 bytes each: address type, then the address.
int debonair_nimble_targets(uint8_t *out, int capacity);
int debonair_nimble_max_targets(void);
