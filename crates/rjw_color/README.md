# `rjw_color` crate

This crate is used to simplify and unify the using of color values.  
本 crate 用于简化和统一颜色值的使用。  

Mainly defines `Color` and `ColorF64` structures.  
主要定义 `Color` 和 `ColorF64` 两种结构体。  

Casting them from or to tuple, list and hex string is included.  
包括颜色值与元组、数组和十六进制字符串的转换。  

Using specific features enables casting them to serialized format, `glam::Vec4`, `wgpu::Color`.  
启用特定的 `feature` 可以启用其与 `glam::Vec4` 和 `wgpu::Color` 等的转换或序列化支持。  

Defined constants from [https://www.w3.org/wiki/CSS/Properties/color/keywords].  
定义了 [https://www.w3.org/wiki/CSS/Properties/color/keywords] 里的颜色常量。  