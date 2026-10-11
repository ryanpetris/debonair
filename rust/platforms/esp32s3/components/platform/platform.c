#include "platform.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "esp_intr_alloc.h"
#include "esp_private/usb_phy.h"
#include "soc/interrupts.h"
#include "soc/usb_dwc_struct.h"
#include "esp_system.h"
#include <stdbool.h>
#include "esp_random.h"
#include "esp_bt.h"
#include "bootloader_random.h"
#include "soc/rtc_cntl_reg.h"
#if CONFIG_DEBONAIR_DEVELOPMENT
#include "esp_private/periph_ctrl.h"
#include "hal/usb_wrap_ll.h"
#include "hal/usb_serial_jtag_ll.h"
#include "esp_rom_sys.h"
#endif

static portMUX_TYPE critical = portMUX_INITIALIZER_UNLOCKED;
void debonair_esp_critical_enter(void) { portENTER_CRITICAL_SAFE(&critical); }
void debonair_esp_critical_exit(void) { portEXIT_CRITICAL_SAFE(&critical); }
void *debonair_esp_current_task(void) { return xTaskGetCurrentTaskHandle(); }
void debonair_esp_wake(void *task) {
    if (xPortInIsrContext()) {
        BaseType_t higher = pdFALSE;
        vTaskNotifyGiveFromISR(task, &higher);
        portYIELD_FROM_ISR(higher);
    } else {
        xTaskNotifyGive(task);
    }
}
void debonair_esp_wait(void) { (void)ulTaskNotifyTake(pdTRUE, portMAX_DELAY); }

esp_err_t debonair_esp_usb_phy(void) {
    static usb_phy_handle_t phy;
    const usb_phy_config_t config = {
        .controller = USB_PHY_CTRL_OTG, .target = USB_PHY_TARGET_INT,
        .otg_mode = USB_OTG_MODE_DEVICE, .otg_speed = USB_PHY_SPEED_FULL,
    };
    return usb_new_phy(&config, &phy);
}
void *debonair_esp_usb_registers(void) { return &USB_DWC; }
esp_err_t debonair_esp_usb_interrupt(void (*handler)(void *), void *context) {
    // Flash writes may mask this interrupt; handler and Embassy code need not
    // live in IRAM. The controller retains packet/FIFO state until it resumes.
    return esp_intr_alloc(ETS_USB_INTR_SOURCE, ESP_INTR_FLAG_LEVEL1, handler, context, NULL);
}

void debonair_esp_random(uint8_t *out, size_t length) {
    // The SAR ADC entropy source must not run beside the radio, which is an entropy source itself.
    bool radio = esp_bt_controller_get_status() == ESP_BT_CONTROLLER_STATUS_ENABLED;
    if (!radio) bootloader_random_enable();
    esp_fill_random(out, length);
    if (!radio) bootloader_random_disable();
}

#if CONFIG_DEBONAIR_DEVELOPMENT
void debonair_esp_bootloader(void) {
    // Embassy owns OTG. End that USB session and return the internal PHY to
    // Serial/JTAG before ROM entry; RTC PHY routing survives a CPU restart.
    USB_DWC.dctl_reg.sftdiscon = 1;
    USB_DWC.gintmsk_reg.val = 0;
    esp_rom_delay_us(20000);
    PERIPH_RCC_ATOMIC() {
        usb_wrap_ll_reset_register();
        usb_wrap_ll_enable_bus_clock(false);
        usb_serial_jtag_ll_enable_bus_clock(true);
        usb_serial_jtag_ll_reset_register();
    }
    REG_CLR_BIT(RTC_CNTL_USB_CONF_REG, RTC_CNTL_USB_PAD_ENABLE);
    usb_serial_jtag_ll_phy_enable_external(false);
    usb_serial_jtag_ll_phy_enable_pad(true);
    // Allow the host to reset the newly attached controller before restarting
    // the CPUs. Leave room for the command's drain of up to 300 ms and CPU
    // restart within its one-second deadline, even if the host does not respond.
    for (unsigned i = 0; i < 500; ++i) {
        if (usb_serial_jtag_ll_get_intraw_mask() & USB_SERIAL_JTAG_INTR_BUS_RESET) {
            break;
        }
        esp_rom_delay_us(1000);
    }
    REG_WRITE(RTC_CNTL_OPTION1_REG, RTC_CNTL_FORCE_DOWNLOAD_BOOT);
    esp_restart();
}
#endif
