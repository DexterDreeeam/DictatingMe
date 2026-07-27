// 每个集成测试二进制只用到 support 的一部分，未使用的项在其它二进制里仍然有用。
#![allow(dead_code)]

pub mod augment;
pub mod corpus;
pub mod harness;
pub mod metrics;
pub mod report;
