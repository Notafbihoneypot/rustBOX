//! Physical frame bookkeeping for the rustBOX kernel.
//!
//! The kernel feeds this the firmware/Xen memory map, then punches out every
//! range it must not hand out (its own image, boot structures, modules).
//! Everything is fixed-capacity and allocation-free so it works before a heap
//! exists, and it has no hardware dependencies so it is unit-tested on the
//! host.
#![no_std]

pub const PAGE_SIZE: u64 = 4096;

/// Half-open physical address range `[start, end)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    pub start: u64,
    pub end: u64,
}

impl Range {
    pub const EMPTY: Range = Range { start: 0, end: 0 };

    pub const fn new(start: u64, end: u64) -> Self {
        Range { start, end }
    }

    /// Range covering `len` bytes from `start`, saturating at the top of the
    /// address space.
    pub const fn from_len(start: u64, len: u64) -> Self {
        Range {
            start,
            end: start.saturating_add(len),
        }
    }

    pub const fn is_empty(&self) -> bool {
        self.start >= self.end
    }

    pub const fn len(&self) -> u64 {
        if self.is_empty() {
            0
        } else {
            self.end - self.start
        }
    }

    /// Largest page-aligned range inside `self` (for memory we may use).
    pub const fn shrink_to_pages(&self) -> Range {
        let start = match self.start.checked_next_multiple_of(PAGE_SIZE) {
            Some(s) => s,
            None => return Range::EMPTY,
        };
        let end = self.end / PAGE_SIZE * PAGE_SIZE;
        if start >= end {
            Range::EMPTY
        } else {
            Range { start, end }
        }
    }

    /// Smallest page-aligned range containing `self` (for memory we must
    /// avoid).
    pub const fn grow_to_pages(&self) -> Range {
        if self.is_empty() {
            return Range::EMPTY;
        }
        let start = self.start / PAGE_SIZE * PAGE_SIZE;
        let end = match self.end.checked_next_multiple_of(PAGE_SIZE) {
            Some(e) => e,
            None => u64::MAX / PAGE_SIZE * PAGE_SIZE,
        };
        Range { start, end }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The fixed-size range table is full.
    TableFull,
}

/// Bump allocator over up to `N` disjoint free ranges.
pub struct FrameAllocator<const N: usize> {
    free: [Range; N],
    len: usize,
}

impl<const N: usize> Default for FrameAllocator<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> FrameAllocator<N> {
    pub const fn new() -> Self {
        FrameAllocator {
            free: [Range::EMPTY; N],
            len: 0,
        }
    }

    /// Adds usable RAM. Partial pages at either end are dropped. The caller
    /// must not add overlapping ranges.
    pub fn add_ram(&mut self, ram: Range) -> Result<(), Error> {
        let ram = ram.shrink_to_pages();
        if ram.is_empty() {
            return Ok(());
        }
        self.push(ram)
    }

    /// Removes `hole` (rounded outwards to whole pages) from the free set.
    pub fn reserve(&mut self, hole: Range) -> Result<(), Error> {
        let hole = hole.grow_to_pages();
        if hole.is_empty() {
            return Ok(());
        }
        let mut i = 0;
        while i < self.len {
            let r = self.free[i];
            if hole.end <= r.start || hole.start >= r.end {
                i += 1;
                continue;
            }
            let left = Range::new(r.start, hole.start.max(r.start));
            let right = Range::new(hole.end.min(r.end), r.end);
            match (left.is_empty(), right.is_empty()) {
                (true, true) => {
                    self.remove(i);
                    continue; // re-examine the range swapped into slot i
                }
                (false, true) => self.free[i] = left,
                (true, false) => self.free[i] = right,
                (false, false) => {
                    // Check capacity before mutating so a failed reserve
                    // never leaves the hole half-applied.
                    if self.len == N {
                        return Err(Error::TableFull);
                    }
                    self.free[i] = left;
                    self.push(right)?;
                }
            }
            i += 1;
        }
        Ok(())
    }

    /// Hands out one 4 KiB frame, lowest free range first.
    pub fn alloc(&mut self) -> Option<u64> {
        while self.len > 0 {
            let r = &mut self.free[0];
            if r.is_empty() {
                self.remove(0);
                continue;
            }
            let frame = r.start;
            r.start += PAGE_SIZE;
            return Some(frame);
        }
        None
    }

    pub fn free_frames(&self) -> u64 {
        self.ranges().iter().map(|r| r.len() / PAGE_SIZE).sum()
    }

    pub fn ranges(&self) -> &[Range] {
        &self.free[..self.len]
    }

    fn push(&mut self, r: Range) -> Result<(), Error> {
        if self.len == N {
            return Err(Error::TableFull);
        }
        self.free[self.len] = r;
        self.len += 1;
        Ok(())
    }

    fn remove(&mut self, i: usize) {
        self.len -= 1;
        self.free[i] = self.free[self.len];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: u64 = 1 << 20;

    #[test]
    fn ram_is_shrunk_and_holes_are_grown_to_page_boundaries() {
        let mut a = FrameAllocator::<4>::new();
        a.add_ram(Range::new(0x1001, 0x5fff)).unwrap();
        assert_eq!(a.ranges(), &[Range::new(0x2000, 0x5000)]);
        a.reserve(Range::new(0x2fff, 0x3001)).unwrap();
        // 0x2000..0x4000 is touched by the hole; only the last page survives.
        assert_eq!(a.ranges(), &[Range::new(0x4000, 0x5000)]);
        assert_eq!(a.free_frames(), 1);
    }

    #[test]
    fn reserving_the_middle_splits_a_range() {
        let mut a = FrameAllocator::<4>::new();
        a.add_ram(Range::new(0, 16 * MIB)).unwrap();
        a.reserve(Range::new(2 * MIB, 3 * MIB)).unwrap();
        let mut got = [Range::EMPTY; 2];
        got.copy_from_slice(a.ranges());
        got.sort_by_key(|r| r.start);
        assert_eq!(got, [Range::new(0, 2 * MIB), Range::new(3 * MIB, 16 * MIB)]);
        assert_eq!(a.free_frames(), 15 * MIB / PAGE_SIZE);
    }

    #[test]
    fn one_hole_can_hit_several_ranges_and_swallow_some() {
        let mut a = FrameAllocator::<4>::new();
        a.add_ram(Range::new(0, MIB)).unwrap();
        a.add_ram(Range::new(2 * MIB, 3 * MIB)).unwrap();
        a.add_ram(Range::new(4 * MIB, 5 * MIB)).unwrap();
        a.reserve(Range::new(MIB / 2, 4 * MIB + MIB / 2)).unwrap();
        let mut got = [Range::EMPTY; 2];
        got.copy_from_slice(a.ranges());
        got.sort_by_key(|r| r.start);
        assert_eq!(
            got,
            [
                Range::new(0, MIB / 2),
                Range::new(4 * MIB + MIB / 2, 5 * MIB)
            ]
        );
    }

    #[test]
    fn alloc_never_returns_reserved_frames_and_runs_dry() {
        let mut a = FrameAllocator::<4>::new();
        a.add_ram(Range::new(0, 4 * PAGE_SIZE)).unwrap();
        a.reserve(Range::new(PAGE_SIZE, 2 * PAGE_SIZE)).unwrap();
        let mut frames = [0u64; 3];
        for f in frames.iter_mut() {
            *f = a.alloc().unwrap();
        }
        frames.sort();
        assert_eq!(frames, [0, 2 * PAGE_SIZE, 3 * PAGE_SIZE]);
        assert_eq!(a.alloc(), None);
        assert_eq!(a.free_frames(), 0);
    }

    #[test]
    fn a_split_that_does_not_fit_fails_without_changing_anything() {
        let mut a = FrameAllocator::<1>::new();
        a.add_ram(Range::new(0, 16 * MIB)).unwrap();
        assert_eq!(a.reserve(Range::new(MIB, 2 * MIB)), Err(Error::TableFull));
        assert_eq!(a.ranges(), &[Range::new(0, 16 * MIB)]);
        assert_eq!(
            a.add_ram(Range::new(32 * MIB, 33 * MIB)),
            Err(Error::TableFull)
        );
    }

    #[test]
    fn ranges_at_the_top_of_the_address_space_do_not_overflow() {
        let top = Range::new(u64::MAX - 10, u64::MAX);
        assert!(top.shrink_to_pages().is_empty());
        assert_eq!(top.grow_to_pages().end % PAGE_SIZE, 0);
        assert_eq!(Range::from_len(u64::MAX - 1, 100).end, u64::MAX);
    }
}
