//! Breakpoints, watch expressions, and step-back execution history.
//!
//! Step-back is emu8086's most distinctive debugger feature. Rather than
//! snapshot the full 1MB memory image on every single-step (wasteful and
//! slow), `History` records only the `(address, old_value)` diffs a step
//! actually touched - fed by `x8086_memory::WriteObserver` - plus the
//! register file before that step. Stepping back replays those diffs in
//! reverse and restores the prior register snapshot.

use std::collections::{BTreeSet, VecDeque};
use x8086_cpu::Registers;

pub mod watch;

#[derive(Debug, Default)]
pub struct Breakpoints {
    addresses: BTreeSet<u32>,
}

impl Breakpoints {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true if the breakpoint is now set (was previously absent).
    pub fn toggle(&mut self, address: u32) -> bool {
        if self.addresses.remove(&address) {
            false
        } else {
            self.addresses.insert(address);
            true
        }
    }

    pub fn is_set(&self, address: u32) -> bool {
        self.addresses.contains(&address)
    }

    pub fn clear_all(&mut self) {
        self.addresses.clear();
    }
}

/// A single step's undo information: the register file immediately
/// before the step ran, and every memory byte it changed (in write
/// order, so undoing replays in reverse).
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub registers_before: Registers,
    pub memory_diffs: Vec<(u32, u8)>,
}

/// A bounded ring of step history. Bounded because an unbounded history
/// would grow without limit on a long-running program; stepping back
/// past the retained window should fail predictably, not corrupt state.
pub struct History {
    entries: VecDeque<HistoryEntry>,
    capacity: usize,
}

impl History {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "history capacity must be positive");
        Self {
            entries: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    pub fn push(&mut self, entry: HistoryEntry) {
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    /// Pop the most recent entry (the one to undo), or `None` if the
    /// history is empty - the caller is already at the earliest known
    /// state and step-back should be a no-op rather than an error.
    pub fn pop(&mut self) -> Option<HistoryEntry> {
        self.entries.pop_back()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_sets_then_clears_a_breakpoint() {
        let mut bp = Breakpoints::new();
        assert!(!bp.is_set(0x100));
        assert!(bp.toggle(0x100));
        assert!(bp.is_set(0x100));
        assert!(!bp.toggle(0x100));
        assert!(!bp.is_set(0x100));
    }

    #[test]
    fn clear_all_removes_every_breakpoint() {
        let mut bp = Breakpoints::new();
        bp.toggle(0x100);
        bp.toggle(0x200);
        bp.clear_all();
        assert!(!bp.is_set(0x100));
        assert!(!bp.is_set(0x200));
    }

    #[test]
    fn history_pops_most_recent_entry_first() {
        let mut history = History::new(10);
        let mut regs_a = Registers::new();
        regs_a.ax = 1;
        let mut regs_b = Registers::new();
        regs_b.ax = 2;

        history.push(HistoryEntry {
            registers_before: regs_a,
            memory_diffs: vec![(0x10, 0x00)],
        });
        history.push(HistoryEntry {
            registers_before: regs_b,
            memory_diffs: vec![(0x20, 0x00)],
        });

        let last = history.pop().expect("should have an entry");
        assert_eq!(last.registers_before.ax, 2);
        let first = history.pop().expect("should have an entry");
        assert_eq!(first.registers_before.ax, 1);
        assert!(history.pop().is_none());
    }

    #[test]
    fn history_is_bounded_and_drops_oldest_entries() {
        let mut history = History::new(2);
        for i in 0..5u16 {
            let mut regs = Registers::new();
            regs.ax = i;
            history.push(HistoryEntry {
                registers_before: regs,
                memory_diffs: vec![],
            });
        }
        assert_eq!(history.len(), 2);
        // Only the last two pushes (ax=3, ax=4) should have survived.
        assert_eq!(history.pop().unwrap().registers_before.ax, 4);
        assert_eq!(history.pop().unwrap().registers_before.ax, 3);
        assert!(history.pop().is_none());
    }

    #[test]
    fn pop_on_empty_history_is_none_not_a_panic() {
        let mut history = History::new(4);
        assert!(history.pop().is_none());
    }
}
