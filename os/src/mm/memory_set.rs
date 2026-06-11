// os/src/mm/memory_set.rs

use core::arch::asm;

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;

use lazy_static::*;
use riscv::register::satp;

use crate::config::{
    MEMORY_END,
    PAGE_SIZE,
    TRAMPOLINE,
    TRAP_CONTEXT,
    USER_STACK_SIZE,
};
use crate::sync::UPSafeCell;

use super::{
    frame_alloc, FrameTracker, PageTable, PageTableEntry, PhysAddr, PhysPageNum,
    PTEFlags, StepByOne, VirtAddr, VirtPageNum, VPNRange,
};

extern "C" {
    fn stext();
    fn etext();
    fn srodata();
    fn erodata();
    fn sdata();
    fn edata();
    fn sbss_with_stack();
    fn ebss();
    fn ekernel();
    fn strampoline();
}

lazy_static! {
    pub static ref KERNEL_SPACE: Arc<UPSafeCell<MemorySet>> = Arc::new(unsafe {
        UPSafeCell::new(MemorySet::new_kernel())
    });
}

pub struct MemorySet {
    page_table: PageTable,
    areas: Vec<MapArea>,
}

pub struct MapArea {
    vpn_range: VPNRange,
    data_frames: BTreeMap<VirtPageNum, FrameTracker>,
    map_type: MapType,
    map_perm: MapPermission,
}

#[derive(Copy, Clone, PartialEq, Debug)]
pub enum MapType {
    Identical,
    Framed,
}

bitflags! {
    pub struct MapPermission: u8 {
        const R = 1 << 1;
        const W = 1 << 2;
        const X = 1 << 3;
        const U = 1 << 4;
    }
}

impl MemorySet {
    /// 创建一个空地址空间
    pub fn new_bare() -> Self {
        Self {
            page_table: PageTable::new(),
            areas: Vec::new(),
        }
    }

    /// 返回当前地址空间对应的 satp token
    pub fn token(&self) -> usize {
        self.page_table.token()
    }

    /// 插入一段 Framed 映射区域
    ///
    /// 这里默认没有地址冲突
    pub fn insert_framed_area(
        &mut self,
        start_va: VirtAddr,
        end_va: VirtAddr,
        permission: MapPermission,
    ) {
        self.push(
            MapArea::new(
                start_va,
                end_va,
                MapType::Framed,
                permission,
            ),
            None,
        );
    }

    /// 插入逻辑段，并可选复制初始化数据
    fn push(&mut self, mut map_area: MapArea, data: Option<&[u8]>) {
        map_area.map(&mut self.page_table);

        if let Some(data) = data {
            map_area.copy_data(&mut self.page_table, data);
        }

        self.areas.push(map_area);
    }

    /// 映射 trampoline 页面
    ///
    /// 注意：trampoline 不放入 areas 中管理
    fn map_trampoline(&mut self) {
        self.page_table.map(
            VirtAddr::from(TRAMPOLINE).into(),
            PhysAddr::from(strampoline as *const () as usize).into(),
            PTEFlags::R | PTEFlags::X,
        );
    }

    /// 激活当前地址空间
    pub fn activate(&self) {
        let satp = self.page_table.token();

        unsafe {
            satp::write(satp);
            asm!("sfence.vma");
        }
    }

    /// 查询虚拟页号对应的页表项
    pub fn translate(&self, vpn: VirtPageNum) -> Option<PageTableEntry> {
        self.page_table.translate(vpn)
    }

    /// 创建内核地址空间
    pub fn new_kernel() -> Self {
        let mut memory_set = Self::new_bare();

        // map trampoline
        memory_set.map_trampoline();

        println!(
            ".text [{:#x}, {:#x})",
            stext as *const () as usize,
            etext as *const () as usize
        );
        println!(
            ".rodata [{:#x}, {:#x})",
            srodata as *const () as usize,
            erodata as *const () as usize
        );
        println!(
            ".data [{:#x}, {:#x})",
            sdata as *const () as usize,
            edata as *const () as usize
        );
        println!(
            ".bss [{:#x}, {:#x})",
            sbss_with_stack as *const () as usize,
            ebss as *const () as usize
        );

        println!("mapping .text section");
        memory_set.push(
            MapArea::new(
                (stext as *const () as usize).into(),
                (etext as *const () as usize).into(),
                MapType::Identical,
                MapPermission::R | MapPermission::X,
            ),
            None,
        );

        println!("mapping .rodata section");
        memory_set.push(
            MapArea::new(
                (srodata as *const () as usize).into(),
                (erodata as *const () as usize).into(),
                MapType::Identical,
                MapPermission::R,
            ),
            None,
        );

        println!("mapping .data section");
        memory_set.push(
            MapArea::new(
                (sdata as *const () as usize).into(),
                (edata as *const () as usize).into(),
                MapType::Identical,
                MapPermission::R | MapPermission::W,
            ),
            None,
        );

        println!("mapping .bss section");
        memory_set.push(
            MapArea::new(
                (sbss_with_stack as *const () as usize).into(),
                (ebss as *const () as usize).into(),
                MapType::Identical,
                MapPermission::R | MapPermission::W,
            ),
            None,
        );

        println!("mapping physical memory");
        memory_set.push(
            MapArea::new(
                (ekernel as *const () as usize).into(),
                MEMORY_END.into(),
                MapType::Identical,
                MapPermission::R | MapPermission::W,
            ),
            None,
        );

        memory_set
    }

    /// 从 ELF 文件创建应用地址空间
    ///
    /// 包括：
    /// 1. trampoline
    /// 2. ELF 中的 LOAD 段
    /// 3. 用户栈
    /// 4. TrapContext
    ///
    /// 返回值：
    /// 1. 应用地址空间
    /// 2. 用户栈顶
    /// 3. 程序入口地址
    pub fn from_elf(elf_data: &[u8]) -> (Self, usize, usize) {
        let mut memory_set = Self::new_bare();

        // map trampoline
        memory_set.map_trampoline();

        let elf = xmas_elf::ElfFile::new(elf_data).unwrap();
        let elf_header = elf.header;

        let magic = elf_header.pt1.magic;
        assert_eq!(magic, [0x7f, 0x45, 0x4c, 0x46], "invalid elf!");

        let ph_count = elf_header.pt2.ph_count();
        let mut max_end_vpn = VirtPageNum(0);

        for i in 0..ph_count {
            let ph = elf.program_header(i).unwrap();

            if ph.get_type().unwrap() == xmas_elf::program::Type::Load {
                let start_va: VirtAddr = (ph.virtual_addr() as usize).into();
                let end_va: VirtAddr =
                    ((ph.virtual_addr() + ph.mem_size()) as usize).into();

                let mut map_perm = MapPermission::U;
                let ph_flags = ph.flags();

                if ph_flags.is_read() {
                    map_perm |= MapPermission::R;
                }
                if ph_flags.is_write() {
                    map_perm |= MapPermission::W;
                }
                if ph_flags.is_execute() {
                    map_perm |= MapPermission::X;
                }

                let map_area = MapArea::new(
                    start_va,
                    end_va,
                    MapType::Framed,
                    map_perm,
                );

                max_end_vpn = map_area.vpn_range.get_end();

                memory_set.push(
                    map_area,
                    Some(
                        &elf.input
                            [ph.offset() as usize
                                ..(ph.offset() + ph.file_size()) as usize],
                    ),
                );
            }
        }

        // map user stack with U flag
        let max_end_va: VirtAddr = max_end_vpn.into();
        let mut user_stack_bottom: usize = max_end_va.into();

        // guard page
        user_stack_bottom += PAGE_SIZE;

        let user_stack_top = user_stack_bottom + USER_STACK_SIZE;

        memory_set.push(
            MapArea::new(
                user_stack_bottom.into(),
                user_stack_top.into(),
                MapType::Framed,
                MapPermission::R | MapPermission::W | MapPermission::U,
            ),
            None,
        );

        // map TrapContext
        memory_set.push(
            MapArea::new(
                TRAP_CONTEXT.into(),
                TRAMPOLINE.into(),
                MapType::Framed,
                MapPermission::R | MapPermission::W,
            ),
            None,
        );

        (
            memory_set,
            user_stack_top,
            elf.header.pt2.entry_point() as usize,
        )
    }
}

impl MapArea {
    pub fn new(
        start_va: VirtAddr,
        end_va: VirtAddr,
        map_type: MapType,
        map_perm: MapPermission,
    ) -> Self {
        let start_vpn: VirtPageNum = start_va.floor();
        let end_vpn: VirtPageNum = end_va.ceil();

        Self {
            vpn_range: VPNRange::new(start_vpn, end_vpn),
            data_frames: BTreeMap::new(),
            map_type,
            map_perm,
        }
    }

    pub fn map_one(
        &mut self,
        page_table: &mut PageTable,
        vpn: VirtPageNum,
    ) {
        let ppn: PhysPageNum;

        match self.map_type {
            MapType::Identical => {
                ppn = PhysPageNum(vpn.0);
            }
            MapType::Framed => {
                let frame = frame_alloc().unwrap();
                ppn = frame.ppn;
                self.data_frames.insert(vpn, frame);
            }
        }

        let pte_flags = PTEFlags::from_bits(self.map_perm.bits).unwrap();
        page_table.map(vpn, ppn, pte_flags);
    }

    #[allow(unused)]
    pub fn unmap_one(
        &mut self,
        page_table: &mut PageTable,
        vpn: VirtPageNum,
    ) {
        if self.map_type == MapType::Framed {
            self.data_frames.remove(&vpn);
        }

        page_table.unmap(vpn);
    }

    pub fn map(&mut self, page_table: &mut PageTable) {
        for vpn in self.vpn_range {
            self.map_one(page_table, vpn);
        }
    }

    #[allow(unused)]
    pub fn unmap(&mut self, page_table: &mut PageTable) {
        for vpn in self.vpn_range {
            self.unmap_one(page_table, vpn);
        }
    }

    /// 将 ELF 中的数据复制到已经映射好的物理页帧中
    ///
    /// data 起始地址是页对齐的，但长度可能不足一页。
    /// 默认新分配的物理页帧已经清零。
    pub fn copy_data(
        &mut self,
        page_table: &mut PageTable,
        data: &[u8],
    ) {
        assert_eq!(self.map_type, MapType::Framed);

        let mut start: usize = 0;
        let mut current_vpn = self.vpn_range.get_start();
        let len = data.len();

        loop {
            let src = &data[start..len.min(start + PAGE_SIZE)];

            let dst = &mut page_table
                .translate(current_vpn)
                .unwrap()
                .ppn()
                .get_bytes_array()[..src.len()];

            dst.copy_from_slice(src);

            start += PAGE_SIZE;

            if start >= len {
                break;
            }

            current_vpn.step();
        }
    }
}

#[allow(unused)]
pub fn remap_test() {
    let kernel_space = KERNEL_SPACE.exclusive_access();

    let mid_text: VirtAddr =
        ((stext as *const () as usize + etext as *const () as usize) / 2).into();
    let mid_rodata: VirtAddr =
        ((srodata as *const () as usize + erodata as *const () as usize) / 2).into();
    let mid_data: VirtAddr =
        ((sdata as *const () as usize + edata as *const () as usize) / 2).into();

    assert_eq!(
        kernel_space
            .page_table
            .translate(mid_text.floor())
            .unwrap()
            .writable(),
        false
    );

    assert_eq!(
        kernel_space
            .page_table
            .translate(mid_rodata.floor())
            .unwrap()
            .writable(),
        false
    );

    assert_eq!(
        kernel_space
            .page_table
            .translate(mid_data.floor())
            .unwrap()
            .executable(),
        false
    );

    println!("remap_test passed!");
}
