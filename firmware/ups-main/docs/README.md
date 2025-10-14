# UPS 主控 MCU 硬件文档

本目录用于记录 UPS 主控（位于 `firmware/ups-main`）的 MCU 引脚分配与外设连接，便于固件实现与联调。

当前包含：

- `mcu_hardware.md`：按原理图抄录的连接清单与“已核实 GPIO 映射”（单一权威表）。
- `pwm_fan_control_circuit_design.md`：风扇 DC 调速方案，技术路线与参考项目一致。
- `datasheets/HUSB305-01.pdf` 与 `datasheets/HUSB305-01.md`：HUSB305‑01 数据手册（Markdown 版含图片）。
