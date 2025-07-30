# RP2040 Embassy Blinky Demo

这是一个使用 Embassy 异步框架和 defmt 日志库的 RP2040 blinky 示例程序。

本项目使用的是来自 https://github.com/IvanLi-CN/embassy 的 Embassy 框架，采用 Rust 2024 版本。

## 功能

- 使用 Embassy 异步执行器
- 通过 defmt 和 RTT 进行调试输出
- 控制 GPIO 25 (Raspberry Pi Pico 板载 LED) 闪烁

## 硬件要求

- Raspberry Pi Pico 或其他 RP2040 开发板
- 调试器 (如 Raspberry Pi Debug Probe 或 Picoprobe)

## 软件要求

安装以下工具：

```bash
# 安装 Rust 目标
rustup target add thumbv6m-none-eabi

# 安装 probe-rs 用于烧录和调试
cargo install probe-rs --features cli
```

## 构建和运行

1. 连接调试器到 RP2040 开发板
2. 构建项目：
   ```bash
   cargo build
   ```
3. 烧录并运行：
   ```bash
   cargo run
   ```

## 调试输出

程序使用 defmt 通过 RTT (Real-Time Transfer) 输出调试信息。运行程序时，你会看到类似以下的输出：

```
INFO  Hello World!
INFO  LED on!
INFO  LED off!
INFO  LED on!
INFO  LED off!
...
```

## 项目结构

- `src/main.rs` - 主程序文件
- `Cargo.toml` - 项目依赖配置
- `memory.x` - 内存布局定义
- `build.rs` - 构建脚本
- `.cargo/config.toml` - Cargo 配置

## 代码说明

程序使用 Embassy 的异步执行器，在主循环中：
1. 点亮 LED (GPIO 25 设为高电平)
2. 等待 500ms
3. 熄灭 LED (GPIO 25 设为低电平)  
4. 等待 500ms
5. 重复

所有操作都通过 defmt 记录日志，便于调试和监控程序运行状态。
