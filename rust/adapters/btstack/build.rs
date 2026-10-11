use std::{env, fs, path::PathBuf, process::Command};

const REVISION: &str = "e38553977a25fb0b55b383c72c289be0975f422c";
const DATABASES: [&str; 3] = ["keyboard", "mouse", "combined"];

fn main() {
    println!("cargo:rerun-if-env-changed=DEBONAIR_BTSTACK_SOURCE");
    println!("cargo:rerun-if-env-changed=DEBONAIR_BTSTACK_LOG");
    println!("cargo:rerun-if-changed=c");
    if env::var_os("CARGO_FEATURE_FFI").is_none() {
        return;
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = env::var_os("DEBONAIR_BTSTACK_SOURCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("../.cache/dependencies/btstack"));
    println!("cargo:rerun-if-changed={}", source.display());
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(&source)
            .args(args)
            .output()
            .expect("inspect BTstack checkout");
        assert!(
            out.status.success(),
            "Fetch the pinned BTstack dependency before building"
        );
        String::from_utf8(out.stdout).unwrap()
    };
    assert_eq!(
        git(&["rev-parse", "HEAD"]).trim(),
        REVISION,
        "BTstack revision differs from the selected pin"
    );
    assert!(
        git(&["status", "--porcelain", "--untracked-files=all"]).is_empty(),
        "BTstack must be unmodified"
    );
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let mut build = cc::Build::new();
    // Each database is its own translation unit: the generated headers share names.
    for name in DATABASES {
        let header = output.join(format!("{name}.h"));
        let generated = Command::new("python3")
            .arg("-B")
            .arg(source.join("tool/compile_gatt.py"))
            .arg(format!("c/{name}.gatt"))
            .arg(&header)
            .output()
            .expect("generate the ATT database");
        assert!(
            generated.status.success(),
            "ATT generation failed: {}",
            String::from_utf8_lossy(&generated.stderr)
        );
        let mut unit = format!(
            "#include \"{name}.h\"\nconst uint8_t *const debonair_db_{name} = profile_data;\n"
        );
        if name == "combined" {
            // The GAP and GATT services come first in every database, so their handles match.
            unit.push_str(
                "const uint16_t debonair_name_handle = \
                 ATT_CHARACTERISTIC_GAP_DEVICE_NAME_01_VALUE_HANDLE;\n\
                 const uint16_t debonair_service_changed_handle = \
                 ATT_CHARACTERISTIC_GATT_SERVICE_CHANGED_01_VALUE_HANDLE;\n\
                 const uint16_t debonair_service_changed_configuration_handle = \
                 ATT_CHARACTERISTIC_GATT_SERVICE_CHANGED_01_CLIENT_CONFIGURATION_HANDLE;\n\
                 const uint16_t debonair_database_hash_handle = \
                 ATT_CHARACTERISTIC_GATT_DATABASE_HASH_01_VALUE_HANDLE;\n",
            );
        }
        let path = output.join(format!("db_{name}.c"));
        fs::write(&path, unit).unwrap();
        build.file(path);
    }
    build
        .std("c11")
        .opt_level_str("s")
        .warnings(true)
        .flag("-ffunction-sections")
        .flag("-fdata-sections")
        .include("c")
        .include(&output)
        .include(source.join("src"))
        .include(source.join("src/ble"))
        .include(source.join("3rd-party/micro-ecc"))
        .include(source.join("3rd-party/rijndael"))
        .define("ENABLE_BLE", None);
    for path in [
        "3rd-party/micro-ecc/uECC.c",
        "3rd-party/rijndael/rijndael.c",
        "src/btstack_crypto.c",
        "src/btstack_hid.c",
        "src/btstack_hid_parser.c",
        "src/btstack_linked_list.c",
        "src/btstack_memory.c",
        "src/btstack_memory_pool.c",
        "src/btstack_run_loop.c",
        "src/btstack_run_loop_base.c",
        "src/btstack_tlv.c",
        "src/btstack_util.c",
        "src/hci.c",
        "src/hci_cmd.c",
        "src/hci_dump.c",
        "src/hci_event.c",
        "src/hci_event_builder.c",
        "src/l2cap.c",
        "src/l2cap_signaling.c",
        "src/ble/att_db.c",
        "src/ble/att_dispatch.c",
        "src/ble/att_server.c",
        "src/ble/sm.c",
        "src/ble/gatt-service/battery_service_server.c",
        "src/ble/gatt-service/device_information_service_server.c",
        "src/ble/gatt-service/hids_device.c",
    ] {
        println!("cargo:rerun-if-changed={}", source.join(path).display());
        build.file(source.join(path));
    }
    // Host tests can print BTstack's log and HCI trace to standard output.
    if env::var_os("DEBONAIR_BTSTACK_LOG").is_some() {
        build
            .define("ENABLE_LOG_INFO", None)
            .define("ENABLE_LOG_ERROR", None)
            .define("ENABLE_PRINTF_HEXDUMP", None)
            .define("DEBONAIR_LOG", None)
            .include(source.join("platform/embedded"))
            .file(source.join("platform/embedded/hci_dump_embedded_stdout.c"));
    }
    build
        .files(["c/runtime.c", "c/hid.c", "c/bond_db.c"])
        .compile("debonair_btstack");
    if env::var("TARGET").unwrap().starts_with("thumb") {
        let output = build
            .get_compiler()
            .to_command()
            .arg("-print-libgcc-file-name")
            .output()
            .expect("compiler runtime path");
        assert!(output.status.success(), "locate ARM compiler runtime");
        let library = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
        assert!(library.is_file(), "ARM compiler runtime missing");
        println!(
            "cargo:rustc-link-search=native={}",
            library.parent().unwrap().display()
        );
        println!("cargo:rustc-link-lib=static=gcc");
    }
}
