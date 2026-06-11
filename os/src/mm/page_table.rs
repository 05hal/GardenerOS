// os/src/mm/page_table.rs

use super::{
    frame_alloc, FrameTracker, PhysAddr, PhysPageNum, StepByOne, VirtAddr,
    VirtPageNum,
};
use alloc::vec;
use alloc::vec::Vec;
use bitflags::*;

bitflags! {
    pub struct PTEFlags: u8 {
        const V = 1 << 0;
        const R = 1 << 1;
        const W = 1 << 2;
        const X = 1 << 3;
        const U = 1 << 4;
        const G = 1 << 5;
        const A = 1 << 6;
        const D = 1 << 7;
    }
}

#[derive(Copy, Clone)]
#[repr(C)]
pub struct PageTableEntry {
    pub bits: usize,
}

impl PageTableEntry {
    pub fn new(ppn: PhysPageNum, flags: PTEFlags) -> Self {
        Self {
            bits: ppn.0 << 10 | flags.bits as usize,
        }
    }

    pub fn empty() -> Self {
        Self {
            bits: 0,
        }
    }

    pub fn ppn(&self) -> PhysPageNum {
        ((self.bits >> 10) & ((1usize << 44) - 1)).into()
    }

    pub fn flags(&self) -> PTEFlags {
        PTEFlags::from_bits(self.bits as u8).unwrap()
    }

    pub fn is_valid(&self) -> bool {
        (self.flags() & PTEFlags::V) != PTEFlags::empty()
    }

    pub fn readable(&self) -> bool {
        (self.flags() & PTEFlags::R) != PTEFlags::empty()
    }

    pub fn writable(&self) -> bool {
        (self.flags() & PTEFlags::W) != PTEFlags::empty()
    }

    pub fn executable(&self) -> bool {
        (self.flags() & PTEFlags::X) != PTEFlags::empty()
    }
}

pub struct PageTable {
    root_ppn: PhysPageNum,
    frames: Vec<FrameTracker>,
}

impl PageTable {
    pub fn new() -> Self {
        let frame = frame_alloc().unwrap();

        Self {
            root_ppn: frame.ppn,
            frames: vec![frame],
        }
    }

    /// 根据已有 satp token 构造页表。
    /// 这里不会拥有物理页帧，因此 frames 为空。
    /// 主要用于内核临时访问用户地址空间。
    pub fn from_token(satp: usize) -> Self {
        Self {
            root_ppn: PhysPageNum::from(satp & ((1usize << 44) - 1)),
            frames: Vec::new(),
        }
    }

    /// 查找页表项；如果中间页表不存在，则自动创建。
    fn find_pte_create(&mut self, vpn: VirtPageNum) -> Option<&mut PageTableEntry> {
        let idxs = vpn.indexes();
        let mut ppn = self.root_ppn;
        let mut result: Option<&mut PageTableEntry> = None;

        for i in 0..3 {
            let pte = &mut ppn.get_pte_array()[idxs[i]];

            if i == 2 {
                result = Some(pte);
                break;
            }

            if !pte.is_valid() {
                let frame = frame_alloc().unwrap();
                *pte = PageTableEntry::new(frame.ppn, PTEFlags::V);
                self.frames.push(frame);
            }

            ppn = pte.ppn();
        }

        result
    }

    /// 只查找页表项；如果中间页表不存在，则返回 None。
    fn find_pte(&self, vpn: VirtPageNum) -> Option<&PageTableEntry> {
        let idxs = vpn.indexes();
        let mut ppn = self.root_ppn;
        let mut result: Option<&PageTableEntry> = None;

        for i in 0..3 {
            let pte = &ppn.get_pte_array()[idxs[i]];

            if i == 2 {
                result = Some(pte);
                break;
            }

            if !pte.is_valid() {
                return None;
            }

            ppn = pte.ppn();
        }

        result
    }

    #[allow(unused)]
    pub fn map(&mut self, vpn: VirtPageNum, ppn: PhysPageNum, flags: PTEFlags) {
        let pte = self.find_pte_create(vpn).unwrap();

        assert!(
            !pte.is_valid(),
            "vpn {:?} is mapped before mapping",
            vpn
        );

        *pte = PageTableEntry::new(ppn, flags | PTEFlags::V);
    }

    #[allow(unused)]
    pub fn unmap(&mut self, vpn: VirtPageNum) {
        let pte = self.find_pte_create(vpn).unwrap();

        assert!(
            pte.is_valid(),
            "vpn {:?} is invalid before unmapping",
            vpn
        );

        *pte = PageTableEntry::empty();
    }

    pub fn translate(&self, vpn: VirtPageNum) -> Option<PageTableEntry> {
        self.find_pte(vpn).map(|pte| *pte)
    }

    pub fn token(&self) -> usize {
        8usize << 60 | self.root_ppn.0
    }
}

/// 将用户地址空间中的一段字节缓冲区翻译成内核可访问的物理页切片。
///
/// token 是用户地址空间的 satp token。
/// ptr 和 len 来自用户程序传入的缓冲区地址和长度。
pub fn translated_byte_buffer(
    token: usize,
    ptr: *const u8,
    len: usize,
) -> Vec<&'static mut [u8]> {
    let page_table = PageTable::from_token(token);
    let mut start = ptr as usize;
    let end = start + len;
    let mut v: Vec<&'static mut [u8]> = Vec::new();

    while start < end {
        let start_va = VirtAddr::from(start);
        let mut vpn = start_va.floor();

        let ppn = page_table
            .translate(vpn)
            .unwrap()
            .ppn();

        vpn.step();

        let mut end_va: VirtAddr = vpn.into();

        if usize::from(end_va) > end {
            end_va = VirtAddr::from(end);
        }

        let start_offset = start_va.page_offset();
        let end_offset = end_va.page_offset();

        if end_offset == 0 {
            v.push(&mut ppn.get_bytes_array()[start_offset..]);
        } else {
            v.push(&mut ppn.get_bytes_array()[start_offset..end_offset]);
        }

        start = usize::from(end_va);
    }

    v
}
