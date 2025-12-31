# UPS 主控 MCU 引脚一览

| Pad | 名称 | GPIO | Net | 说明 |
| --- | ---- | ---- | --- | ---- |
| 1 | LNA_IN | — | — | 射频输入（天线匹配） |
| 2 | VDD3P3 | — | — | 数字 3V3 电源 |
| 3 | VDD3P3 | — | — | 数字 3V3 电源 |
| 4 | CHIP_PU | — | CHIP_EN | 芯片使能（高有效） |
| 5 | GPIO0 | 0 | BTN_CENTER | 五向中键（内部上拉） |
| 6 | GPIO1 | 1 | BTN_UP | 五向上（内部上拉） |
| 7 | GPIO2 | 2 | BTN_RIGHT | 五向右（内部上拉） |
| 8 | GPIO3 | 3 | — | 启动绑带：JTAG 信号源选择（usb_to_jtag/pad_to_jtag），复位采样；不建议占用 |
| 9 | GPIO4 | 4 | BTN_DOWN | 五向下（内部上拉） |
| 10 | GPIO5 | 5 | BTN_LEFT | 五向左（内部上拉） |
| 11 | GPIO6 | 6 | RESET# | 外设复位（TCA6408A RESET#，外部上拉；建议内部上拉） |
| 12 | GPIO7 | 7 | INT | I2C 从设备中断（开漏，低有效） |
| 13 | GPIO8 | 8 | SDA | I2C 主机 SDA |
| 14 | GPIO9 | 9 | SCL | I2C 主机 SCL |
| 15 | GPIO10 | 10 | DC | SPI 屏 DC（GC9D01） |
| 16 | GPIO11 | 11 | MOSI | SPI 屏 MOSI |
| 17 | GPIO12 | 12 | SCLK | SPI 屏 SCLK |
| 18 | GPIO13 | 13 | CS | SPI 屏 CS |
| 19 | GPIO14 | 14 | RES | SPI 屏复位 |
| 20 | VDD3P3_RTC | — | — | RTC 3V3 电源 |
| 21 | XTAL_32K_P | — | — | 32.768 kHz 晶振 P（本板未用） |
| 22 | XTAL_32K_N | — | — | 32.768 kHz 晶振 N（本板未用） |
| 23 | GPIO17 | 17 | — | 未连接 |
| 24 | GPIO18 | 18 | — | 未连接 |
| 25 | GPIO19 | 19 | ESP_DM | USB D−（原生 USB，经 R2 串联；使用 USB 时不建议复用） |
| 26 | GPIO20 | 20 | ESP_DP | USB D+（原生 USB，经 R3 串联；使用 USB 时不建议复用） |
| 27 | GPIO21 | 21 | USB2_PG | 来自 HUSB305‑01 STAT，开漏，低有效 |
| 28 | SPICS1 | — | — | 内置 Flash/PSRAM 通道（保留，不建议使用） |
| 29 | VDD_SPI | — | — | 内部 SPI 供电 |
| 30 | SPIHD | — | — | 内置 Flash/PSRAM 专用（保留，不建议使用） |
| 31 | SPIWP | — | — | 内置 Flash/PSRAM 专用（保留，不建议使用） |
| 32 | SPICS0 | — | — | 内置 Flash/PSRAM 专用（保留，不建议使用） |
| 33 | SPICLK | — | — | 内置 Flash/PSRAM 专用（保留，不建议使用） |
| 34 | SPIQ | — | — | 内置 Flash/PSRAM 专用（保留，不建议使用） |
| 35 | SPID | — | — | 内置 Flash/PSRAM 专用（保留，不建议使用） |
| 36 | SPICLK_N | — | — | Octal 外设差分时钟 N（本板未用） |
| 37 | SPICLK_P | — | — | Octal 外设差分时钟 P（本板未用） |
| 38 | GPIO33 | 33 | — | 未连接 |
| 39 | GPIO34 | 34 | — | 未连接 |
| 40 | GPIO35 | 35 | — | 未连接 |
| 41 | GPIO36 | 36 | — | 未连接 |
| 42 | GPIO37 | 37 | — | 未连接 |
| 43 | GPIO38 | 38 | BUZZER | 蜂鸣器（无源，2.7 kHz PWM） |
| 44 | MTCK | 39 | FAN_EN | 风扇使能（由 MTCK 改作 GPIO39 输出；默认 JTAG 脚位） |
| 45 | MTDO | 40 | FAN_PWM | 风扇 PWM（由 MTDO 改作 GPIO40 PWM 输出；默认 JTAG 脚位） |
| 46 | VDD3P3_CPU | — | — | CPU 3V3 电源 |
| 47 | MTDI | 41 | — | JTAG TDI（未连接，量产不建议占用；JTAG 默认引脚） |
| 48 | MTMS | 42 | — | JTAG TMS（未连接，量产不建议占用；JTAG 默认引脚） |
| 49 | U0TXD | — | TX1 | UART0 TX（默认日志/下载口，建议保留；测试点） |
| 50 | U0RXD | — | RX1 | UART0 RX（默认日志/下载口，建议保留；测试点） |
| 51 | GPIO45 | 45 | — | 启动绑带：VDD_SPI 电压配置相关；不建议使用（复位采样） |
| 52 | GPIO46 | 46 | — | 启动绑带/日志：UART Boot 打印控制；不建议使用（复位采样） |
| 53 | XTAL_N | — | — | 40 MHz 晶振 N |
| 54 | XTAL_P | — | — | 40 MHz 晶振 P |
| 55 | VDDA | — | — | 模拟电源 |
| 56 | VDDA | — | — | 模拟电源 |
| 57 | GND(EP) | — | — | 裸焊盘接地 |

注：USB2_PG 为 HUSB305‑01 的 STAT 输出逻辑，详见 `docs/datasheets/HUSB305-01.md`。
