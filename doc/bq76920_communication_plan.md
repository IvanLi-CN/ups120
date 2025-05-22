# BQ76920 通信实现方案

**MCU 型号：STM32G031G8U6**

本文档详细规划了使用 STM32G031C8U6 微控制器与 BQ76920 进行通信的实现方案，旨在满足 5S1P 磷酸铁锂电池组的应用需求。方案包括详细的设备连接、关键信息获取、参数配置、周期性数据监测以及必要的保护机制实现。重点在于提高通信效率和数据处理速度，确保电池管理系统的稳定性和可靠性。

## 1. 硬件连接

BQ76920 与 STM32G031G8U6 微控制器主要通过 I2C 接口通信，同时需要连接电池组的电芯电压采样线、电流采样线以及控制和状态引脚。以下是详细的引脚连接说明：

**通信导线连接：**

* **I2C 通信：**
    * STM32G031G8U6 的 I2C SDA 引脚 (PB7) 连接到 BQ76920 的 SDA 引脚 (Pin 4)。
    * STM32G031G8U6 的 I2C SCL 引脚 (PB6) 连接到 BQ76920 的 SCL 引脚 (Pin 5)。
* **唤醒信号 (TS1)：**
    * STM32G031G8U6 的 GPIO 输出引脚 (PA0) 连接到 BQ76920 的 TS1 引脚 (Pin 6)。**TS1 引脚作为输出引脚，MCU 在进行其他操作前，通过 PA0 输出高电平来激活 BQ76920。**
* **报警信号 (ALERT)：**
    * **必须实现。** STM32G031G8U6 的中断输入引脚 (PA1) 连接到 BQ76920 的 ALERT 引脚 (Pin 20)。用于接收 BQ76920 的报警中断信号。配置 STM32 的中断，以便在 ALERT 引脚变为低电平时触发中断服务程序，及时响应 BQ76920 发出的报警信号。

**引脚连接总结：**

| BQ76920 引脚 | STM32G031G8U6 引脚 | 说明 |
|---|---|---|
| SDA (Pin 4) | PB7 | I2C 数据通信引脚，用于双向数据传输 |
| SCL (Pin 5) | PB6 | I2C 时钟信号引脚，用于同步 I2C 数据传输 |
| TS1 (Pin 6) | PA0 | BQ76920 唤醒信号控制引脚，STM32 通过 PA0 输出高电平激活 BQ76920 |
| ALERT (Pin 20) | PA1 | BQ76920 报警信号接收引脚，STM32 通过 PA1 接收 BQ76920 的报警中断信号 |
| VC0 (Pin 17) |  | 电芯 1 负极 |
| VC1 (Pin 16) |  | 电芯 1 正极 / 电芯 2 负极 |
| VC2 (Pin 15) |  | 电芯 2 正极 / 电芯 3 负极 |
| VC3 (Pin 14) |  | 电芯 3 正极 / 电芯 4 负极 |
| VC4 (Pin 13) |  | 电芯 4 正极 / 电芯 5 负极 |
| VC5 (Pin 12) |  | 电芯 5 正极 |
| SRP (Pin 18) |  | 电流采样正极 |
| SRN (Pin 19) |  | 电流采样负极 |
| VSS (Pin 3) |  | 负电源 |
| BAT (Pin 10) |  | 电池正极 |
| REGSRC (Pin 9) |  | 稳压器电源 |
| REGOUT (Pin 8) |  | 稳压器输出 |
| CAP1 (Pin 7) |  | 电容连接 |

## 2. 实现方案步骤

1.  **初始化：**
    * 微控制器初始化 I2C 外设。
    * 微控制器等待用户按下连接到 TS1 的按钮。
2.  **唤醒设备：**
    * 用户按下按钮，TS1 引脚被拉高，触发 BQ76920 从 SHIP 模式唤醒。
    * 微控制器等待 BQ76920 完成启动序列 (tBOOTREADY 约 10ms) 并准备好进行 I2C 通信 (tI2CSTARTUP 约 1ms)。
3.  **连接与基本配置：**
    * STM32G031G8U6 尝试通过 I2C 与 BQ76920 通信 (例如，读取 SYS_STAT 寄存器，I2C 地址为 0x08)。检查是否收到 ACK 信号以确认连接成功。如果未收到 ACK 信号，则表明连接存在问题，需要检查硬件连接或 I2C 初始化配置。
    * 读取 ADCGAIN (0x50, 0x59) 和 ADCOFFSET (0x51) 寄存器，获取用于电压和温度转换的校准值。
    * 向 CC_CFG (0x0B) 寄存器写入 0x19，以优化库仑计数器性能。
    * 通过写入 SYS_CTRL1 (0x04) 寄存器设置 ADC_EN (Bit 4) 为 1，启用电压和温度 ADC 读取。
    * 通过写入 SYS_CTRL2 (0x05) 寄存器设置 CC_EN (Bit 6) 为 1，启用库仑计数器持续读取。
4.  **配置保护设置：**
    * **OCD/SCD (过流保护/短路保护)：** 您希望将保护电流设置为 200mA 进行测试。使用 3mΩ 的 Rsns，200mA 对应的压降为 0.2A * 0.003Ω = 0.6mV。根据数据手册，BQ76920 的硬件过流保护阈值最低为 8mV (OCD) 和 22mV (SCD)，远高于 0.6mV。这意味着无法通过硬件保护直接实现 200mA 的电流限制。
        * **建议：** 将硬件 OCD/SCD 保护设置为最低有效阈值，以启用硬件保护功能。对于 200mA 的测试电流限制，建议在微控制器软件中通过读取库仑计数器 (CC) 的值来实时监测电流，并在电流超过 200mA 时采取相应措施 (例如控制放电 FET)。
        * 将 PROTECT1 (0x06) 寄存器的 RSNS (Bit 7) 设置为 0，SCD_D1:0 (Bits 4-3) 和 SCD_T2:0 (Bits 2-0) 设置为 0x0，以使用最低阈值和默认延迟。写入 PROTECT1 寄存器 0x00。
        * 将 PROTECT2 (0x07) 寄存器的 OCD_D2:0 (Bits 6-4) 和 OCD_T3:0 (Bits 3-0) 设置为 0x0，以使用最低阈值和默认延迟。写入 PROTECT2 寄存器 0x00。
    * **OV/UV (过压保护/欠压保护)：** 根据 5S 磷酸铁锂电池的特性，设置合适的过压和欠压保护阈值。例如，单节电池过压阈值可设为 3.6V，欠压阈值可设为 2.5V。需要根据读取到的 ADCGAIN 和 ADCOFFSET 值计算对应的 OV_TRIP (0x09) 和 UV_TRIP (0x0A) 寄存器值。
        * **示例计算 (使用典型值 GAIN=382 μV/LSB, OFFSET=0 mV)：**
            * 单节 3.6V 对应的 ADC 值 ≈ (3600 mV - OFFSET) / GAIN ≈ 3600 / 0.382 ≈ 9424。根据数据手册格式，OV_TRIP 寄存器值应为 0x4D。
            * 单节 2.5V 对应的 ADC 值 ≈ (2500 mV - OFFSET) / GAIN ≈ 2500 / 0.382 ≈ 6545。根据数据手册格式，UV_TRIP 寄存器值应为 0x99。
        * 将 PROTECT3 (0x08) 寄存器的 OV_D1:0 (Bits 5-4) 和 UV_D1:0 (Bits 7-6) 设置为合适的延迟 (例如默认的 1s)，RSVD (Bits 3-0) 设置为 0x0。写入 PROTECT3 寄存器 0x00。
        * 写入计算出的 OV_TRIP (0x09) 和 UV_TRIP (0x0A) 寄存器值。
    * **FET 控制 (可选)：** 如果需要控制充放电 FET，根据需要设置 SYS_CTRL2 (0x05) 寄存器的 CHG_ON (Bit 0) 和 DSG_ON (Bit 1)。
5.  **周期性读取和打印信息：**
    * 微控制器进入一个循环，每秒执行一次以下操作：
        * 读取各节电芯电压：读取 VC1_HI/LO (0x0C/0x0D) 到 VC5_HI/LO (0x14/0x15) 寄存器。使用读取到的 ADCGAIN 和 ADCOFFSET 将 ADC 值转换为电压。
        * 读取电池组总电压：读取 BAT_HI/LO (0x2A/0x2B) 寄存器。使用读取到的 ADCGAIN 和 ADCOFFSET 以及电芯数量 (5) 将 ADC 值转换为总电压。
        * 读取电池组电流：读取 CC_HI/LO (0x32/0x33) 寄存器。将 16 位值转换为电压 (使用 8.44 μV/LSB)，然后除以 Rsns (3mΩ) 转换为电流。
        * 读取温度：读取 TS1_HI/LO (0x2C/0x2D) 寄存器。根据 SYS_CTRL1 的 TEMP_SEL 位判断是热敏电阻还是芯片内部温度。如果是热敏电阻，将 ADC 值转换为电阻 (使用数据手册公式 5)，再根据热敏电阻数据手册或查找表转换为温度。
        * 读取故障状态：读取 SYS_STAT (0x00) 寄存器，检查 OCD, SCD, OV, UV 等故障状态位。
        * 将读取到的电池电压、总电压、电流、温度和故障状态信息打印输出。
        * 通过向 SYS_STAT (0x00) 寄存器对应的位写入 1 来清除已检测到的故障状态位。

## 3. 交互序列图

\`\`\`mermaid
sequenceDiagram
    participant Microcontroller
    participant BQ76920

    Microcontroller->>BQ76920: (初始状态：SHIP 模式)
    User->>Microcontroller: 按下按钮 (TS1 拉高)
    Microcontroller->>BQ76920: TS1 高电平信号
    BQ76920->>BQ76920: 启动 (10ms)
    BQ76920->>BQ76920: I2C 准备就绪 (1ms)
    Microcontroller->>BQ76920: 尝试 I2C 通信 (例如，读取 SYS_STAT @ 0x00)
    BQ76920-->>Microcontroller: ACK (连接成功)

    Microcontroller->>BQ76920: 读取 ADCGAIN (0x50, 0x59)
    BQ76920-->>Microcontroller: ADCGAIN 值
    Microcontroller->>BQ76920: 读取 ADCOFFSET (0x51)
    BQ76920-->>Microcontroller: ADCOFFSET 值

    Microcontroller->>BQ76920: 写入 0x19 到 CC_CFG (0x0B)
    BQ76920-->>Microcontroller: ACK
    Microcontroller->>BQ76920: 写入 SYS_CTRL1 (0x04) 启用 ADC (设置 Bit 4)
    BQ76920-->>Microcontroller: ACK
    Microcontroller->>BQ76920: 写入 SYS_CTRL2 (0x05) 启用 CC (设置 Bit 6)
    BQ76920-->>Microcontroller: ACK

    Microcontroller->>BQ76920: 写入 PROTECT1 (0x06) 配置 SCD (例如，0x00 使用最低阈值/延迟)
    BQ76920-->>Microcontroller: ACK
    Microcontroller->>BQ76920: 写入 PROTECT2 (0x07) 配置 OCD (例如，0x00 使用最低阈值/延迟)
    BQ76920-->>Microcontroller: ACK
    Microcontroller->>BQ76920: 写入 OV_TRIP (0x09) (计算值，例如 0x4D 对应 3.6V)
    BQ76920-->>Microcontroller: ACK
    Microcontroller->>BQ76920: 写入 UV_TRIP (0x0A) (计算值，例如 0x99 对应 2.5V)
    BQ76920-->>Microcontroller: ACK
    Microcontroller->>BQ76920: 写入 PROTECT3 (0x08) 配置 OV/UV 延迟 (例如，0x00 对应 1s 延迟)
    BQ76920-->>Microcontroller: ACK

    loop 每秒
        Microcontroller->>BQ76920: 读取 VC1_HI/LO (0x0C/0x0D) 到 VC5_HI/LO (0x14/0x15)
        BQ76920-->>Microcontroller: 电芯电压 ADC 值
        Microcontroller->>BQ76920: 读取 BAT_HI/LO (0x2A/0x2B)
        BQ76920-->>Microcontroller: 总电压 ADC 值
        Microcontroller->>BQ76920: 读取 CC_HI/LO (0x32/0x33)
        BQ76920-->>Microcontroller: 电流 ADC 值
        Microcontroller->>BQ76920: 读取 TS1_HI/LO (0x2C/0x2D)
        BQ76920-->>Microcontroller: 温度 ADC 值
        Microcontroller->>BQ76920: 读取 SYS_STAT (0x00)
        BQ76920-->>Microcontroller: 状态寄存器值

        Microcontroller->>Microcontroller: 转换 ADC 值为电压/电流/温度
        Microcontroller->>Microcontroller: 检查 SYS_STAT 中的故障位
        Microcontroller->>User: 打印电池/故障信息

        Microcontroller->>BQ76920: 写入 1 清除 SYS_STAT (0x00) 中的故障位
        BQ76920-->>Microcontroller: ACK
    end
\`\`\`
