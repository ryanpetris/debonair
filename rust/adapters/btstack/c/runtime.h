#ifndef DEBONAIR_BTSTACK_RUNTIME_H
#define DEBONAIR_BTSTACK_RUNTIME_H
#include <stdint.h>
#include "btstack_chipset.h"

typedef struct {
    void *context;
    uint32_t (*time_ms)(void *);
    void (*wake)(void *);
    int (*can_send)(void *);
    int (*send)(void *, uint8_t, const uint8_t *, uint16_t);
    void (*fatal)(void *);
} debonair_runtime_callbacks;

void debonair_runtime_init(const debonair_runtime_callbacks *, const btstack_chipset_t *);
void debonair_runtime_poll(void);
int32_t debonair_runtime_timeout(void);
void debonair_runtime_receive(uint8_t kind, uint8_t *data, uint16_t size);
void debonair_runtime_sent(void);

#endif
