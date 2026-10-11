#pragma once
#ifndef __ASSEMBLER__
#include <stddef.h>
void *debonair_radio_alloc(size_t size);
void debonair_radio_free(void *ptr);
#define cyw43_malloc debonair_radio_alloc
#define cyw43_free debonair_radio_free
#endif
