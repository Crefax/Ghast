//! API misuse returns errors instead of panicking (builds with any feature set).

use ghast_inventory::{ContainerClick, Inventory, Outgoing, Version, VersionNotEnabled};

#[test]
fn new_fails_for_a_version_built_without_its_feature() {
    for v in [
        Version::V1_8,
        Version::V1_12,
        Version::V1_16_5,
        Version::V26_1,
    ] {
        match Inventory::new(v) {
            Ok(inv) => {
                assert!(v.is_enabled());
                assert_eq!(inv.version(), v);
            }
            Err(e) => {
                assert!(!v.is_enabled());
                assert_eq!(e, VersionNotEnabled(v));
            }
        }
    }
}

#[test]
#[allow(unused_variables)]
fn a_26_1_click_has_no_legacy_packet() {
    let c = Outgoing::ContainerClick(ContainerClick {
        window_id: 0,
        state_id: 1,
        slot: 36,
        button: 0,
        mode: 0,
        changed: Vec::new(),
        carried: None,
        predicted: false,
        action: 1,
    });
    #[cfg(feature = "v1_8")]
    assert!(c.to_v1_8().is_none());
    #[cfg(feature = "v1_12")]
    assert!(c.to_v1_12_2().is_none());
    #[cfg(feature = "v1_16_5")]
    assert!(c.to_v1_16_5().is_none());
}
