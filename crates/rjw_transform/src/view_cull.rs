//! （已合并）可见性判定不再单独成 trait。
//!
//! `ViewCull` 曾是「为 3D 预留」的抽象，但仓库内唯一实现是 `Camera2D`，没有第二个实现方。
//! 按 `docs/API_DESIGN.md` §5「简并表」，可见性判定并入 [`crate::Camera2D::view_aabb`]：
//!
//! - 保守世界 AABB：`cam.view_aabb()`
//! - 判定：`rect.intersects(&cam.view_aabb())`
//!
//! 本文件不再被 `lib.rs` 声明为模块（避免死 `pub` 面），仅留作迁移说明。
