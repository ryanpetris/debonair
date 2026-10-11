#pragma once
#include <stddef.h>
#include <stdint.h>
#include "esp_err.h"
#include "sdkconfig.h"

void debonair_esp_critical_enter(void);
void debonair_esp_critical_exit(void);
void *debonair_esp_current_task(void);
void debonair_esp_wake(void *task);
void debonair_esp_wait(void);
esp_err_t debonair_esp_usb_phy(void);
void *debonair_esp_usb_registers(void);
esp_err_t debonair_esp_usb_interrupt(void (*handler)(void *), void *context);
// Fills `out` from the hardware random number generator. The generator is truly random only while
// an entropy source runs: the radio once the Bluetooth controller is enabled, and otherwise the
// SAR ADC source, which this enables for the call.
void debonair_esp_random(uint8_t *out, size_t length);
#if CONFIG_DEBONAIR_DEVELOPMENT
void debonair_esp_bootloader(void) __attribute__((noreturn));
#endif
#if CONFIG_BT_NIMBLE_ENABLED
#include "nimble_hid.h"
#endif
#if CONFIG_BT_CONTROLLER_ONLY
esp_err_t debonair_esp_controller_start(void (*sent)(void), int (*received)(uint8_t *, uint16_t));
esp_err_t debonair_esp_controller_send(const uint8_t *data, uint16_t length);
#endif
