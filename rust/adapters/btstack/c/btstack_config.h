#ifndef DEBONAIR_BTSTACK_CONFIG_H
#define DEBONAIR_BTSTACK_CONFIG_H

#define ENABLE_BTSTACK_ASSERT
#define ENABLE_SOFTWARE_AES128
#define ENABLE_MICRO_ECC_FOR_LE_SECURE_CONNECTIONS
#define ENABLE_LE_PERIPHERAL
#define ENABLE_LE_SECURE_CONNECTIONS
#define ENABLE_LE_PRIVACY_ADDRESS_RESOLUTION
#define ENABLE_LE_DATA_LENGTH_EXTENSION
#define HCI_ACL_CHUNK_SIZE_ALIGNMENT 4
/* The Pico transport has a 1024-byte packet including its indicator/header. */
#define HCI_ACL_PAYLOAD_SIZE 1016
#define HCI_OUTGOING_PRE_BUFFER_SIZE 4
#define HCI_RESET_RESEND_TIMEOUT_MS 100
/* One Target at a time, plus a connection being refused. */
#define MAX_NR_HCI_CONNECTIONS 2
#define MAX_NR_L2CAP_CHANNELS 2
#define MAX_NR_L2CAP_SERVICES 1
#define MAX_NR_SM_LOOKUP_ENTRIES 4
#define MAX_NR_WHITELIST_ENTRIES 4
#define MAX_NR_LE_DEVICE_DB_ENTRIES 8
/* A subscription tag for each of the 8 subscriptions of each of the 8 bonds in bond_db.c. */
#define NVN_NUM_GATT_SERVER_CCC 64
#define MAX_ATT_DB_SIZE 0

#endif
