//! A tiny wire writer for hand-built packet bodies.
#![allow(dead_code)]

use bytes::Bytes;
use ghast_protocol::RawPacket;

#[derive(Default)]
pub struct W(pub Vec<u8>);

impl W {
    pub fn new() -> Self {
        W(Vec::new())
    }
    pub fn varint(mut self, v: i32) -> Self {
        let mut v = v as u32;
        loop {
            if v & !0x7F == 0 {
                self.0.push(v as u8);
                return self;
            }
            self.0.push((v as u8 & 0x7F) | 0x80);
            v >>= 7;
        }
    }
    pub fn u8(mut self, v: u8) -> Self {
        self.0.push(v);
        self
    }
    pub fn i8(self, v: i8) -> Self {
        self.u8(v as u8)
    }
    pub fn bool(self, v: bool) -> Self {
        self.u8(u8::from(v))
    }
    pub fn i16(mut self, v: i16) -> Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    pub fn i32(mut self, v: i32) -> Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    pub fn i64(mut self, v: i64) -> Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    pub fn f32(mut self, v: f32) -> Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    pub fn f64(mut self, v: f64) -> Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    pub fn uuid(mut self, v: u128) -> Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    pub fn str(self, s: &str) -> Self {
        let mut w = self.varint(s.len() as i32);
        w.0.extend_from_slice(s.as_bytes());
        w
    }
    pub fn bytes(mut self, b: &[u8]) -> Self {
        self.0.extend_from_slice(b);
        self
    }
    pub fn raw(self, id: i32) -> RawPacket {
        RawPacket::new(id, Bytes::from(self.0))
    }
}

/// `count` player_info "add" entries body (1.8 and 1.16 share the layout).
pub fn player_add(uuid: u128, name: &str, with_props: bool) -> W {
    let mut w = W::new().varint(0).varint(1).uuid(uuid).str(name);
    if with_props {
        w = w
            .varint(2)
            .str("textures")
            .str(&"x".repeat(500))
            .bool(true)
            .str(&"s".repeat(300))
            .str("other")
            .str("v")
            .bool(false);
    } else {
        w = w.varint(0);
    }
    w.varint(1).varint(42).bool(true).str("{\"text\":\"Bob\"}")
}
