# UPS 主控 MCU 硬件文档

本目录用于记录 UPS 主控（位于 `firmware/ups-main`）的 MCU 引脚分配与外设连接，便于固件实现与联调。

当前包含：

- `mcu_hardware.md`：按原理图抄录的连接清单与“已核实 GPIO 映射”（单一权威表）。
- `fan_control_spec.md`：两线风扇调速规范（硬件+软件设计、验收流程）。
- `fan_control_requirements.md`：当前分支的任务拆解与验收条目。
- `archive/pwm_fan_control_circuit_design.md`：三线风扇参考方案（来自其他项目，仅供历史参考）。
- `datasheets/HUSB305-01.pdf` 与 `datasheets/HUSB305-01.md`：HUSB305‑01 数据手册（Markdown 版含图片）。
