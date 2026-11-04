//! Places where the generated code follows vanilla instead of minecraft-data
//! (configured in `codegen/protocol/src/main.rs`, `VersionSpec::presence`).

use bytes::{Bytes, BytesMut};
use ghast_protocol::codec::decode_exact;
use ghast_protocol::{Encode, EncodeError};

/// 1.8.9 spawn object: the velocity follows iff `objectData > 0`.
#[cfg(feature = "v1_8")]
#[test]
fn v1_8_spawn_entity_velocity_iff_data_positive() {
    use ghast_protocol::v1_8::play::clientbound::{ClientboundPlayPacket, SpawnEntity, id};
    use ghast_protocol::v1_8::types::Vec3i16;
    use ghast_protocol::{DecodeError, PacketSet, RawPacket};

    let base = |object_data, velocity| SpawnEntity {
        entity_id: 300,
        r#type: 70,
        x: -32,
        y: 2048,
        z: 64,
        pitch: 0,
        yaw: -128,
        object_data,
        velocity,
    };
    // entityId 300 = ac 02, type 70, x, y, z, pitch, yaw, then objectData.
    let head: &[u8] = &[
        0xac, 0x02, 0x46, 0xff, 0xff, 0xff, 0xe0, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x40,
        0x00, 0x80,
    ];
    let cases: Vec<(i32, Option<Vec3i16>, Vec<u8>)> = vec![
        (-1, None, vec![0xff, 0xff, 0xff, 0xff]),
        (i32::MIN, None, vec![0x80, 0x00, 0x00, 0x00]),
        (0, None, vec![0x00, 0x00, 0x00, 0x00]),
        (
            1,
            Some(Vec3i16 {
                x: -1,
                y: 0,
                z: 32767,
            }),
            vec![0x00, 0x00, 0x00, 0x01, 0xff, 0xff, 0x00, 0x00, 0x7f, 0xff],
        ),
        (
            5,
            Some(Vec3i16 {
                x: 1,
                y: -2,
                z: 800,
            }),
            vec![0x00, 0x00, 0x00, 0x05, 0x00, 0x01, 0xff, 0xfe, 0x03, 0x20],
        ),
    ];
    for (data, velocity, tail) in cases {
        let p = base(data, velocity);
        let mut w = BytesMut::new();
        p.encode(&mut w).unwrap();
        let mut want = head.to_vec();
        want.extend_from_slice(&tail);
        assert_eq!(&w[..], &want[..], "objectData {data}");
        assert_eq!(
            decode_exact::<SpawnEntity>(&Bytes::from(want.clone())).unwrap(),
            p,
            "objectData {data}"
        );
        // Strict: one extra byte is rejected, one byte short is an error.
        let mut longer = want.clone();
        longer.push(0);
        assert!(decode_exact::<SpawnEntity>(&Bytes::from(longer)).is_err());
        want.pop();
        assert!(decode_exact::<SpawnEntity>(&Bytes::from(want)).is_err());
    }
    // A negative objectData followed by velocity bytes is 6 trailing bytes, not a velocity.
    let mut bytes = head.to_vec();
    bytes.extend_from_slice(&[0xff, 0xff, 0xff, 0xff, 0, 1, 0, 2, 0, 3]);
    assert!(matches!(
        decode_exact::<SpawnEntity>(&Bytes::from(bytes.clone())),
        Err(DecodeError::TrailingBytes(6))
    ));
    // Same through the packet-set dispatch used by the client.
    let raw = RawPacket::new(id::SPAWN_ENTITY, Bytes::from(bytes));
    assert!(matches!(
        ClientboundPlayPacket::decode_raw(&raw),
        Err(DecodeError::TrailingBytes(6))
    ));
    let mut ok = head.to_vec();
    ok.extend_from_slice(&[0xff, 0xff, 0xff, 0xff]);
    let raw = RawPacket::new(id::SPAWN_ENTITY, Bytes::from(ok));
    assert_eq!(
        ClientboundPlayPacket::decode_raw(&raw).unwrap(),
        ClientboundPlayPacket::SpawnEntity(base(-1, None))
    );
    // Encoding refuses a velocity the discriminant says is absent, and vice versa.
    let mut w = BytesMut::new();
    let bad = base(-1, Some(Vec3i16 { x: 0, y: 0, z: 0 }));
    assert!(matches!(
        bad.encode(&mut w),
        Err(EncodeError::SwitchMismatch(_))
    ));
    let mut w = BytesMut::new();
    assert!(matches!(
        base(7, None).encode(&mut w),
        Err(EncodeError::SwitchMismatch(_))
    ));
}

/// 1.16.5 always carries the velocity (not a switch at all).
#[cfg(feature = "v1_16_5")]
#[test]
fn v1_16_5_spawn_entity_velocity_always_present() {
    use ghast_protocol::types::Uuid;
    use ghast_protocol::v1_16_5::play::clientbound::SpawnEntity;
    use ghast_protocol::v1_16_5::types::Vec3i16;

    for object_data in [-1, 0, 5] {
        let p = SpawnEntity {
            entity_id: 1,
            object_uuid: Uuid::from_u128(7),
            r#type: 2,
            x: 0.5,
            y: 64.0,
            z: -0.5,
            pitch: 0,
            yaw: 0,
            object_data,
            velocity: Vec3i16 { x: 1, y: 2, z: 3 },
        };
        let mut w = BytesMut::new();
        p.encode(&mut w).unwrap();
        // 1 + 16 + 1 + 24 + 2 + 4 + 6 bytes
        assert_eq!(w.len(), 54);
        assert_eq!(&w[w.len() - 6..], &[0, 1, 0, 2, 0, 3]);
        assert_eq!(decode_exact::<SpawnEntity>(&w.freeze()).unwrap(), p);
    }
}
