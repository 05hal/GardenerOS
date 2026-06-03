use core::arch::global_asm;

global_asm!(include_str!("trap.S"));

mod context;
pub use context::TrapContext;

use riscv::register::{
    mtvec::TrapMode,
    stvec,
    scause::{self, Trap, Exception, Interrupt},
    stval,
};

use crate::syscall::syscall;
use crate::task::{exit_current_and_run_next, suspend_current_and_run_next};
use crate::timer::set_next_trigger;

/// 初始化 Trap 向量
pub fn init() {
    extern "C" { fn __alltraps(); }
    unsafe {
        stvec::write(__alltraps as *const () as usize, TrapMode::Direct);
    }
}

/// 使能时钟中断
pub fn enable_timer_interrupt() {
    unsafe { riscv::register::sie::set_stimer(); }
}

/// Trap 处理函数
#[unsafe(no_mangle)]
pub fn trap_handler(cx: &mut TrapContext) -> &mut TrapContext {
    let scause = scause::read();
    let stval = stval::read();

    match scause.cause() {
        // 用户环境调用 (系统调用)
        Trap::Exception(Exception::UserEnvCall) => {
            cx.sepc += 4;
            cx.x[10] = syscall(cx.x[17], [cx.x[10], cx.x[11], cx.x[12]]) as usize;
        }

        // 页错误或存储异常
        Trap::Exception(Exception::StoreFault) |
        Trap::Exception(Exception::StorePageFault) => {
            println!(
                "[kernel] PageFault in application, bad addr = {:#x}, bad instruction = {:#x}, core dumped.",
                stval, cx.sepc
            );
            exit_current_and_run_next();
        }

        // 非法指令
        Trap::Exception(Exception::IllegalInstruction) => {
            println!("[kernel] IllegalInstruction in application, core dumped.");
            exit_current_and_run_next();
        }

        // Supervisor Timer 中断：实现抢占式调度
        Trap::Interrupt(Interrupt::SupervisorTimer) => {
            set_next_trigger();                // 设置下一次触发
            suspend_current_and_run_next();    // 挂起当前任务并切换到下一个
        }

        // 其他未支持异常
        _ => {
            panic!(
                "Unsupported trap {:?}, stval = {:#x}!",
                scause.cause(),
                stval
            );
        }
    }

    cx
}
