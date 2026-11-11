//! API misuse returns errors instead of panicking (builds with any feature set).

use ghast_inventory::{Inventory, Version};

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
                assert_eq!(e.0, v);
            }
        }
    }
}

#[test]
#[cfg(feature = "v26_1")]
#[allow(unused_variables)]
fn a_26_1_click_has_no_legacy_packet() {
    use ghast_inventory::{Click, Outgoing};
    let mut inv = Inventory::new(Version::V26_1).unwrap();
    inv.tick();
    assert_eq!(inv.open_inventory(), Ok(None));
    inv.tick();
    let c = inv
        .click(Click::Pickup {
            slot: 36,
            right: false,
        })
        .unwrap();
    assert!(matches!(c, Outgoing::ContainerClick(_)));
    #[cfg(feature = "v1_8")]
    assert!(c.to_v1_8().is_none());
    #[cfg(feature = "v1_12")]
    assert!(c.to_v1_12_2().is_none());
    #[cfg(feature = "v1_16_5")]
    assert!(c.to_v1_16_5().is_none());
}
