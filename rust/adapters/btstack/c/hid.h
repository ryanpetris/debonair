#ifndef DEBONAIR_HID_H
#define DEBONAIR_HID_H
#include <stdbool.h>
#include <stdint.h>

// Events copied by the Rust owner before the callback returns.
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
};

typedef struct {
    uint8_t kind;
    // Success flag, protocol mode or address type, depending on kind.
    uint8_t code;
    uint8_t report_id;
    // For a successful PAIRING_COMPLETE, the bond table entry that holds the pairing, or -1 for
    // none.
    int8_t entry;
    // Most significant byte first.
    uint8_t address[6];
    uint16_t length;
    const uint8_t *data;
} debonair_event;

typedef void (*debonair_emit)(void *context, const debonair_event *event);

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

// Sets up the HID peripheral and powers on the controller. Call once, after the runtime.
int debonair_hid_start(const debonair_hid_config *config, void *context, debonair_emit emit);
int debonair_hid_set_name(const char *name, uint16_t length);
void debonair_hid_set_security(uint8_t passkey);
void debonair_hid_set_pairing(bool enabled);
// Answers a passkey request. accept zero rejects the pairing.
int debonair_hid_passkey(int accept, uint32_t passkey);
int debonair_hid_disconnect(void);
// Writes up to capacity paired Targets as 7 bytes each: address type, then the address.
int debonair_hid_targets(uint8_t *out, int capacity);
int debonair_hid_max_targets(void);
int debonair_hid_forget(uint8_t address_type, const uint8_t *address);
void debonair_hid_set_battery(uint8_t level);
// Whether a report can be sent now. When not, a CAN_SEND event follows once it can.
int debonair_hid_can_send(void);
int debonair_hid_send(uint8_t channel, uint8_t report_id, const uint8_t *data, uint16_t length);

#endif
