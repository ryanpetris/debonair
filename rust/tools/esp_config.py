"""ESP32-S3 board validation and isolated native SDK configuration."""
import hashlib
import json
from pathlib import Path

BACKENDS = {"btstack": "BT_CONTROLLER_ONLY", "esp-nimble": "BT_NIMBLE_ENABLED"}


def normalize(config, profile):
    from firmware_config import integer, indicator
    if (config["package"] != "esp32s3" or config["radio"] is not None or
            config["xosc_hz"] != 40000000 or config["radio_backend"] != "esp-idf"):
        raise ValueError("ESP32-S3 uses its internal radio and a 40 MHz crystal")
    backend = config["bluetooth_backend"]
    if backend not in BACKENDS or config["usb_backend"] != "embassy" or config["storage_backend"] != "littlefs":
        raise ValueError("ESP32-S3 requires a selected Bluetooth host, embassy USB and LittleFS")
    flash = integer(config["flash_bytes"], 2097152, 16777216, "flash_bytes")
    if flash & (flash - 1):
        raise ValueError("ESP32-S3 flash capacity must be a power of two")
    offset = integer(config["firmware_bytes"], 2097152, flash - 131072, "firmware_bytes", 65536)
    storage = flash - offset
    led = indicator(config["mcu_led"], 48, "mcu_led")
    if led is not None:
        if led in (19, 20, *range(22, 38)):
            raise ValueError("LED pin is absent, reserved for flash, or used by native USB")
    physical = {k: config[k] for k in ("chip", "package", "flash_bytes", "xosc_hz", "mcu_led")}
    digest = hashlib.sha256(json.dumps(physical, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    return config | {"profile": profile, "target": "xtensa-esp32s3-espidf",
                     "storage_bytes": storage, "hardware_digest": digest, "storage_offset": flash - storage}


def generate(config, out):
    from firmware_config import storage_identity_bytes
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    partitions = out / "partitions.csv"
    # Storage is the guard block holding the layout identity, then LittleFS. There is no NVS.
    partitions.write_text(f"""# Name,Type,SubType,Offset,Size
phy_init,data,phy,0xf000,0x1000
factory,app,factory,0x10000,{config['storage_offset'] - 0x10000:#x}
layout,data,0x40,{config['storage_offset']:#x},0x1000
storage,data,0x83,{config['storage_offset'] + 0x1000:#x},{config['storage_bytes'] - 0x1000:#x}
""")
    values = {
        "IDF_TARGET": "esp32s3", "BT_ENABLED": True, "BT_CONTROLLER_ENABLED": True,
        "DEBONAIR_DEVELOPMENT": config["profile"] == "development",
        "ESP_CONSOLE_NONE": True, "ESP_CONSOLE_SECONDARY_NONE": True,
        "USJ_ENABLE_USB_SERIAL_JTAG": True, "LOG_DEFAULT_LEVEL_NONE": True,
        "BOOTLOADER_LOG_LEVEL_NONE": True,
        # The partition table has no NVS partition. Calibrate the PHY fully at every boot rather
        # than loading and storing calibration data in NVS.
        "ESP_PHY_CALIBRATION_AND_DATA_STORAGE": False,
        "FREERTOS_HZ": 1000, "ESP_MAIN_TASK_STACK_SIZE": 65536,
        f"ESPTOOLPY_FLASHSIZE_{config['flash_bytes'] // 1048576}MB": True,
        "PARTITION_TABLE_CUSTOM": True, "PARTITION_TABLE_CUSTOM_FILENAME": str(partitions.resolve()),
    }
    values.update({choice: backend == config["bluetooth_backend"] for backend, choice in BACKENDS.items()})
    if config["bluetooth_backend"] == "esp-nimble":
        # A peripheral with one connection. Security keys are kept by the application; NimBLE's
        # own store holds only subscriptions, in RAM.
        values.update({"BT_NIMBLE_ROLE_PERIPHERAL": True, "BT_NIMBLE_ROLE_BROADCASTER": True,
                       "BT_NIMBLE_ROLE_CENTRAL": False, "BT_NIMBLE_ROLE_OBSERVER": False,
                       "BT_NIMBLE_GATT_SERVER": True, "BT_NIMBLE_SECURITY_ENABLE": True,
                       "BT_NIMBLE_SM_SC": True, "BT_NIMBLE_SM_LEGACY": True,
                       "BT_NIMBLE_MAX_CONNECTIONS": 1, "BT_NIMBLE_MAX_BONDS": 8,
                       "BT_NIMBLE_NVS_PERSIST": False, "BT_NIMBLE_GAP_DEVICE_NAME_MAX_LEN": 31,
                       "BT_NIMBLE_HID_SERVICE": False, "BT_NIMBLE_STATIC_TO_DYNAMIC": False,
                       # Up to 7 subscriptions for each of 8 bonds; an overflow would remove a bond.
                       "BT_NIMBLE_MAX_CCCDS": 64,
                       # Loads paired Targets' identity keys into the controller.
                       "BT_NIMBLE_HS_PVCY": True,
                       # Secure Connections and the application's event callback run on this task.
                       "BT_NIMBLE_HOST_TASK_STACK_SIZE": 8192})
    def encode(value):
        return ("y" if value else "n") if isinstance(value, bool) else json.dumps(value)
    # esp-idf-sys watches this defaults file, but not extra components' C sources
    # or transitive headers. Make their changes trigger its native build too.
    components = Path(__file__).resolve().parents[1] / "platforms/esp32s3/components"
    digest = hashlib.sha256()
    digest.update(partitions.read_bytes())
    for path in sorted(components.rglob("*")):
        if path.is_file():
            digest.update(str(path.relative_to(components)).encode())
            digest.update(path.read_bytes())
    defaults = f"# Project native sources: {digest.hexdigest()}\n"
    defaults += "".join(f"CONFIG_{key}={encode(value)}\n" for key, value in values.items())
    (out / "sdkconfig.defaults").write_text(defaults)
    (out / "board.rs").write_text(f"""pub const DEFAULT_ADAPTER_NAME: &str = {json.dumps(config["default_adapter_name"], ensure_ascii=False)};
pub const HARDWARE: &str = {json.dumps(config['name'])};
pub const STORAGE_START: u32 = {config['storage_offset']};
pub const STORAGE_END: u32 = {config['flash_bytes']};
pub const STORAGE_IDENTITY: [u8; 32] = {storage_identity_bytes(config)!r};
pub const MCU_LED: Option<(u8, bool)> = {('Some((' + str(config['mcu_led']['pin']) + ', ' + str(config['mcu_led']['active_low']).lower() + '))') if config['mcu_led'] else 'None'};
""")
