#ifndef DEBONAIR_BOND_DB_H
#define DEBONAIR_BOND_DB_H
// BTstack's LE device database and TLV, kept in RAM. The Rust owner fills the database from the
// saved bonds before the stack starts and saves the entries the stack changes; nothing here
// touches storage.
#include <stdbool.h>
#include <stdint.h>
#include "btstack_tlv.h"

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

// A bond in the form the Rust owner saves. BTstack keeps one set of keys, in `peer`.
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

// Empties the database and sets the identity roots the TLV answers with, or none when both are
// NULL: BTstack's security manager then makes them from the controller's random numbers and
// stores them through the TLV. Call before the stack starts.
void debonair_bonds_reset(const uint8_t ir[16], const uint8_t er[16]);
// Copies identity roots the security manager made since the last call. Returns 0 when it made
// none.
int debonair_bonds_take_roots(uint8_t ir[16], uint8_t er[16]);
// Adds a saved bond, oldest first. Returns its entry, or -1 when the database is full.
int debonair_bonds_import(const debonair_bond *bond);
// Copies entry `slot`. Returns 0 for an empty entry.
int debonair_bonds_export(int slot, debonair_bond *out);
// The entries that changed since the last call, as one bit per entry: returns those whose keys
// changed or that were removed, sets `added` to those that hold a new pairing and `updated` to
// those whose feature report, database identifier or subscriptions changed.
uint32_t debonair_bonds_take(uint32_t *added, uint32_t *updated);
const btstack_tlv_t *debonair_bonds_tlv(void);

// A pairing completed in entry `slot`, which becomes the newest pairing.
void debonair_bonds_paired(int slot);
void debonair_bonds_set_feature(int slot, uint8_t value);
// Whether entry `slot` holds a feature report, which is copied to `value`.
bool debonair_bonds_feature(int slot, uint8_t *value);
void debonair_bonds_set_database(int slot, const uint8_t *id, uint8_t length);
// Whether entry `slot` holds the database identifier `id`.
bool debonair_bonds_database_is(int slot, const uint8_t *id, uint8_t length);
// Forgets the subscriptions of every bond whose database identifier is not `id`: their handles
// belong to another database.
void debonair_bonds_keep_subscriptions(const uint8_t *id, uint8_t length);

#endif
