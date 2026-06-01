#![no_std]
#![feature(linkage)]

#[macro_use]
pub mod console;
mod syscall;
mod lang_items;

use syscall::*;

pub fn write(fd: usize, buf: &[u8]) -> isize {
    sys_write(fd, buf)
}

pub fn exit(exit_code: i32) -> isize {
    sys_exit(exit_code)
}

fn clear_bss() {
    unsafe extern "C" {
        fn start_bss();
        fn end_bss();
    }
    let start_bss_ptr = start_bss as *const () as usize;
    let end_bss_ptr = end_bss as *const () as usize;
    (start_bss_ptr..end_bss_ptr)
        .for_each(|a| unsafe { (a as *mut u8).write_volatile(0) });
}
#[unsafe(no_mangle)]
#[unsafe(link_section = ".text.entry")]
pub extern "C" fn _start() -> ! {
    clear_bss();
    exit(main());
    panic!("unreachable after sys_exit!");
}

#[linkage = "weak"]
#[unsafe(no_mangle)]
fn main() -> i32 {
    panic!("Cannot find main!");
}
pub fn yield_() -> isize { sys_yield() }
