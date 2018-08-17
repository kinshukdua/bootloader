use core::{
    slice,
    fmt,
    ops::{
        Deref,
        DerefMut,
    },
};

use usize_conversions::usize_from;
use x86_64::{
    VirtAddr,
    PhysAddr,
    structures::paging::{
        PhysFrameRange,
        PhysFrame,
    },
};

pub struct BootInfo<'data> {
    pub p4_table_addr: u64,
    pub memory_map: MemoryMap,
    pub package: &'data [u8],
}

impl<'data> BootInfo<'data> {
    pub(crate) fn new(p4_table_addr: u64, memory_map: MemoryMap, package: &'data [u8]) -> Self {
        BootInfo {
            p4_table_addr,
            memory_map,
            package,
        }
    }
}

pub struct MemoryMap {
    entries: [MemoryRegion; 32],
    // u64 instead of usize so that the structure layout is platform
    // independent
    next_entry_index: u64,
}

impl MemoryMap {
    pub fn new() -> Self {
        MemoryMap {
            entries: [MemoryRegion::empty(); 32],
            next_entry_index: 0,
        }
    }

    pub fn add_region(&mut self, region: MemoryRegion) {
        self.entries[self.next_entry_index()] = region;
        self.next_entry_index += 1;
        self.sort();
    }

    pub fn sort(&mut self) {
        use core::cmp::Ordering;

        self.entries.sort_unstable_by(|r1, r2| {
            if r1.range.is_empty() {
                Ordering::Greater
            } else if r2.range.is_empty() {
                Ordering::Less
            } else {
                
                let ordering = r1.range
                    .start
                    .cmp(&r2.range.start);
                
                if ordering == Ordering::Equal {
                    r1.range
                    .end
                    .cmp(&r2.range.end)
                } else {
                    ordering   
                }
            }
        });
        if let Some(first_zero_index) = self.entries.iter().position(|r| r.range.is_empty()) {
            self.next_entry_index = first_zero_index as u64;
        }
    }

    fn next_entry_index(&self) -> usize {
        self.next_entry_index as usize
    }
}

impl Deref for MemoryMap {
    type Target = [MemoryRegion];

    fn deref(&self) -> &Self::Target {
        &self.entries[0..self.next_entry_index()]
    }
}

impl DerefMut for MemoryMap {
    fn deref_mut(&mut self) -> &mut Self::Target {
        let next_index = self.next_entry_index();
        &mut self.entries[0..next_index]
    }
}

impl fmt::Debug for MemoryMap {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryRegion {
    pub range: PhysFrameRange,
    pub region_type: MemoryRegionType,
}

impl MemoryRegion {
    pub fn empty() -> Self {
        MemoryRegion {
            range: PhysFrame::range(
                PhysFrame::containing_address(PhysAddr::new(0)),
                PhysFrame::containing_address(PhysAddr::new(0)),
            ),
            region_type: MemoryRegionType::Empty,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum MemoryRegionType {
    /// free RAM
    Usable,
    /// used RAM
    InUse,
    /// unusable
    Reserved,
    /// ACPI reclaimable memory
    AcpiReclaimable,
    /// ACPI NVS memory
    AcpiNvs,
    /// Area containing bad memory
    BadMemory,
    /// kernel memory
    Kernel,
    /// kernel stack memory
    KernelStack,
    /// memory used by page tables
    PageTable,
    /// memory used by the bootloader
    Bootloader,
    /// frame at address zero
    ///
    /// (shouldn't be used because it's easy to make mistakes related to null pointers)
    FrameZero,
    /// an empty region with size 0
    Empty,
    /// used for storing the boot information
    BootInfo,
    /// used for storing the supplied package
    Package,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
struct E820MemoryRegion {
    pub start_addr: u64,
    pub len: u64,
    pub region_type: u32,
    pub acpi_extended_attributes: u32,
}

impl From<E820MemoryRegion> for MemoryRegion {
    fn from(region: E820MemoryRegion) -> MemoryRegion {
        let region_type = match region.region_type {
            1 => MemoryRegionType::Usable,
            2 => MemoryRegionType::Reserved,
            3 => MemoryRegionType::AcpiReclaimable,
            4 => MemoryRegionType::AcpiNvs,
            5 => MemoryRegionType::BadMemory,
            t => panic!("invalid region type {}", t),
        };
        MemoryRegion {
            range: PhysFrame::range(
                PhysFrame::containing_address(PhysAddr::new(region.start_addr)),
                PhysFrame::containing_address(PhysAddr::new(region.start_addr + region.len)),
            ),
            region_type,
        }
    }
}

pub(crate) fn create_from(memory_map_addr: VirtAddr, entry_count: u64) -> MemoryMap {
    let memory_map_start_ptr = usize_from(memory_map_addr.as_u64()) as *const E820MemoryRegion;
    let e820_memory_map =
        unsafe { slice::from_raw_parts(memory_map_start_ptr, usize_from(entry_count)) };

    let mut memory_map = MemoryMap::new();
    for region in e820_memory_map {
        memory_map.add_region(MemoryRegion::from(*region));
    }

    memory_map.sort();

    let mut iter = memory_map.iter_mut().peekable();
    while let Some(region) = iter.next() {
        if let Some(next) = iter.peek() {
            if region.range.end > next.range.start {
                if region.region_type == MemoryRegionType::Usable {
                    region.range.end = next.range.start;
                } else {
                    panic!("two non-usable regions overlap: {:?} {:?}", region, next);
                }
            }
        }
    }

    memory_map
}
