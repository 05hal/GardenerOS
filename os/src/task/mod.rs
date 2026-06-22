use alloc::sync::Arc;
mod pid;
pub use pid::{PidHandle, KernelStack};
mod processor;
use processor::{take_current_task, schedule};
pub use processor::current_task;
mod manager;
pub use manager::{add_task, fetch_task};
pub mod context;
mod switch;
mod task;
pub use context::TaskContext;
use crate::loader::{get_num_app, get_app_data};
use crate::trap::TrapContext;
use crate::sync::UPSafeCell;
use lazy_static::*;
use switch::__switch;
use task::{TaskControlBlock, TaskStatus};
use alloc::vec::Vec;
use crate::loader::get_app_data_by_name;

lazy_static! {
    pub static ref INITPROC: Arc<TaskControlBlock> = Arc::new(
        TaskControlBlock::new(get_app_data_by_name("initproc").unwrap())
    );
}

pub fn add_initproc() {
    add_task(INITPROC.clone());
}
pub fn run_first_task() {
    processor::run_tasks();
}




pub fn suspend_current_and_run_next() {
    let task = take_current_task().unwrap();
    let mut task_inner = task.inner_exclusive_access();
    task_inner.task_status = TaskStatus::Ready;
    let task_cx_ptr = &mut task_inner.task_cx as *mut TaskContext;
    drop(task_inner);
    add_task(task);
    schedule(task_cx_ptr);
}

pub fn exit_current_and_run_next(exit_code: i32) {
    let task = take_current_task().unwrap();
    let mut task_inner = task.inner_exclusive_access();
    task_inner.task_status = TaskStatus::Zombie;
    task_inner.exit_code = exit_code;

    {
        let mut initproc_inner = INITPROC.inner_exclusive_access();
        for child in task_inner.children.iter() {
            child.inner_exclusive_access().parent = Some(Arc::downgrade(&INITPROC));
            initproc_inner.children.push(child.clone());
        }
    }

    task_inner.children.clear();
    task_inner.memory_set.recycle_data_pages();
    drop(task_inner);
    drop(task);

    let mut _unused = TaskContext::zero_init();
    schedule(&mut _unused as *mut _);
}

pub fn current_user_token() -> usize {
    current_task().unwrap().inner_exclusive_access().get_user_token()
}

pub fn current_trap_cx() -> &'static mut TrapContext {
    current_task().unwrap().inner_exclusive_access().get_trap_cx()
}



pub fn run_tasks() {
    run_first_task();
}
