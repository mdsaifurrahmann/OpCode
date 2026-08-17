//! Flat 1MB memory model for 8086 real-mode addressing.
//!
//! The 8086 has no MMU: a segment:offset pair resolves to a 20-bit
//! physical address (`segment * 16 + offset`), and that address wraps
//! at the 1MB boundary. This crate owns that resolution and the raw
//! byte storage; it does not know about registers or instructions.

/// Total addressable memory: 2^20 bytes (1MB).
pub const MEMORY_SIZE: usize = 1 << 20;

/// Notified on every byte write, so the debugger can build undo/step-back
/// history without the memory model needing to know what a "snapshot" is.
pub trait WriteObserver {
    fn on_write(&mut self, address: u32, old_value: u8, new_value: u8);
}

pub struct Memory {
    bytes: Box<[u8]>,
    observer: Option<Box<dyn WriteObserver>>,
}

impl Memory {
    pub fn new() -> Self {
        Self {
            bytes: vec![0u8; MEMORY_SIZE].into_boxed_slice(),
            observer: None,
        }
    }

    pub fn set_observer(&mut self, observer: Box<dyn WriteObserver>) {
        self.observer = Some(observer);
    }

    pub fn clear_observer(&mut self) {
        self.observer = None;
    }

    /// Resolve a real-mode segment:offset pair to a 20-bit physical
    /// address, wrapping at the 1MB boundary as real 8086 hardware does.
    pub fn resolve(segment: u16, offset: u16) -> u32 {
        (((segment as u32) << 4).wrapping_add(offset as u32)) & 0x000F_FFFF
    }

    pub fn read_u8(&self, address: u32) -> u8 {
        self.bytes[(address as usize) & (MEMORY_SIZE - 1)]
    }

    pub fn write_u8(&mut self, address: u32, value: u8) {
        let index = (address as usize) & (MEMORY_SIZE - 1);
        let old_value = self.bytes[index];
        self.bytes[index] = value;
        if old_value != value {
            if let Some(observer) = self.observer.as_mut() {
                observer.on_write(address & 0x000F_FFFF, old_value, value);
            }
        }
    }

    /// Little-endian 16-bit read; the high byte wraps independently at
    /// the 1MB boundary, matching real 8086 behavior.
    pub fn read_u16(&self, address: u32) -> u16 {
        let low = self.read_u8(address);
        let high = self.read_u8(address.wrapping_add(1));
        u16::from_le_bytes([low, high])
    }

    pub fn write_u16(&mut self, address: u32, value: u16) {
        let [low, high] = value.to_le_bytes();
        self.write_u8(address, low);
        self.write_u8(address.wrapping_add(1), high);
    }
}

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_computes_paragraph_shifted_address() {
        assert_eq!(Memory::resolve(0x07C0, 0x0010), 0x07C10);
        assert_eq!(Memory::resolve(0x0000, 0x0000), 0x00000);
    }

    #[test]
    fn resolve_wraps_at_one_megabyte_boundary() {
        assert_eq!(
            Memory::resolve(0xFFFF, 0xFFFF),
            (0xFFFF0u32 + 0xFFFF) & 0xFFFFF
        );
    }

    #[test]
    fn u8_round_trip_read_write() {
        let mut mem = Memory::new();
        mem.write_u8(0x1234, 0xAB);
        assert_eq!(mem.read_u8(0x1234), 0xAB);
        assert_eq!(mem.read_u8(0x1235), 0x00);
    }

    #[test]
    fn u16_round_trip_is_little_endian() {
        let mut mem = Memory::new();
        mem.write_u16(0x2000, 0x1234);
        assert_eq!(mem.read_u8(0x2000), 0x34);
        assert_eq!(mem.read_u8(0x2001), 0x12);
        assert_eq!(mem.read_u16(0x2000), 0x1234);
    }

    #[test]
    fn write_observer_is_notified_with_old_and_new_values() {
        let mut mem = Memory::new();
        // Observer capture happens through a shared handle so the test can
        // inspect events after the fact.
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        struct SharedRecorder(std::rc::Rc<std::cell::RefCell<Vec<(u32, u8, u8)>>>);
        impl WriteObserver for SharedRecorder {
            fn on_write(&mut self, address: u32, old_value: u8, new_value: u8) {
                self.0.borrow_mut().push((address, old_value, new_value));
            }
        }
        mem.set_observer(Box::new(SharedRecorder(events.clone())));

        mem.write_u8(0x0100, 0x42);
        mem.write_u8(0x0100, 0x42); // no-op write: value unchanged, no event
        mem.write_u8(0x0100, 0x43);

        assert_eq!(
            *events.borrow(),
            vec![(0x0100, 0x00, 0x42), (0x0100, 0x42, 0x43)]
        );
    }
}
