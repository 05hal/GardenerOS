#![no_std]
#![no_main]
#[macro_use]
extern crate user_lib;

#[unsafe(no_mangle)]
fn main() -> i32 {
    println!("Into Test 03sum!");

    let mut sum: i32 = 0;

    for i in 1..=100 {
        sum += i;
    }

    println!("sum from 1 to 100 is {}", sum);

    if sum == 5050 {
        println!("03sum passed!");
        0
    } else {
        println!("03sum failed!");
        -1
    }
}
