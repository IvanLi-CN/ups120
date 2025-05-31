# BQ25730 配置重构计划

## 目标
重构 `bq25730` 驱动库，使用静态配置结构体 (`Config`) 替代运行时电阻检测，提高初始化效率和可配置性。

## 主要变更

### 1. 创建配置结构体
```rust
// bq25730/src/data_types.rs
pub struct Config {
    pub charge_option0: ChargeOption0,
    pub charge_option1: ChargeOption1,
    pub charge_current: u16,      // REG0x03/02h
    pub charge_voltage: u16,      // REG0x05/04h
    pub input_voltage: u16,       // REG0x0B/0Ah
    pub vsys_min: u16,            // REG0x0D/0Ch
    pub iin_host: u16,            // REG0x0F/0Eh
    // 其他必要寄存器...
}
```

### 2. 实现 Default trait
```rust
impl Default for Config {
    fn default() -> Self {
        Config {
            charge_option0: ChargeOption0::default(),
            charge_option1: ChargeOption1::default(),
            charge_current: 0x0000,  // 默认充电电流 = 0
            charge_voltage: match cell_count {
                1 => 0x1068,        // 4.2V
                2 => 0x20D0,        // 8.4V
                3 => 0x3138,        // 12.6V
                4 => 0x41A0,        // 16.8V
                5 => 0x5208,        // 21.0V
                _ => 0x41A0,        // 默认4节
            },
            input_voltage: 0x1E00,  // 12.8V
            vsys_min: match cell_count {
                1 => 0x2400,        // 3.6V
                2 => 0x4200,        // 6.6V
                3 => 0x5C00,        // 9.2V
                4 => 0x7B00,        // 12.3V
                5 => 0x9A00,        // 15.4V
                _ => 0x7B00,        // 默认4节
            },
            iin_host: 0x2000,      // 3.2A
        }
    }
}
```

### 3. 修改 Bq25730 结构体
```rust
// bq25730/src/lib.rs
pub struct Bq25730<I2C> {
    i2c: I2C,
    address: u8,
    config: Config,  // 替换原来的 rsns_bat, rsns_ac 等字段
    // 其他字段保持不变...
}
```

### 4. 更新 new() 方法
```rust
pub fn new(
    i2c: I2C,
    address: u8,
    cell_count: u8,
    config: Config  // 新增配置参数
) -> Self {
    Bq25730 {
        i2c,
        address,
        config,
        // 其他初始化...
    }
}
```

### 5. 重构 init() 方法（优化寄存器写入）
```rust
pub async fn init(&mut self) -> Result<(), Error<E>> {
    // 分组连续寄存器写入以提高效率
    // 第一组：连续寄存器 (0x02-0x05)
    self.write_registers_bulk(&[
        (Register::ChargeCurrentLSB, self.config.charge_current as u8),
        (Register::ChargeCurrentMSB, (self.config.charge_current >> 8) as u8),
        (Register::ChargeVoltageLSB, self.config.charge_voltage as u8),
        (Register::ChargeVoltageMSB, (self.config.charge_voltage >> 8) as u8),
    ]).await?;
    
    // 第二组：连续寄存器 (0x0A-0x0F)
    self.write_registers_bulk(&[
        (Register::InputVoltage, self.config.input_voltage as u8),
        (Register::VSYS_MIN_LSB, self.config.vsys_min as u8),
        (Register::VSYS_MIN_MSB, (self.config.vsys_min >> 8) as u8),
        (Register::IIN_HOST_LSB, self.config.iin_host as u8),
        (Register::IIN_HOST_MSB, (self.config.iin_host >> 8) as u8),
    ]).await?;
    
    // 选项寄存器（不连续，单独写入）
    self.write_register(ChargeOption0::from(self.config.charge_option0.bits())).await?;
    self.write_register(ChargeOption1::from(self.config.charge_option1.bits())).await?;

    // 其他初始化逻辑...
}
```

## 寄存器写入优化说明
1. **批量写入连续寄存器**：
   - 将物理地址连续的寄存器分组批量写入（如 0x02-0x05 和 0x0A-0x0F）
   - 减少I2C通信开销，提高初始化速度
   - 使用 `write_registers_bulk` 方法实现

2. **单独写入非连续寄存器**：
   - 选项寄存器（0x00-01h 和 0x30-31h）地址不连续
   - 保持单独写入以确保正确性

3. **性能平衡**：
   - 初始化通常只执行一次，性能影响有限
   - 清晰度和可维护性优先于微优化

## 实施步骤
1. 更新 `data_types.rs` 添加 `Config` 结构体和 `Default` 实现
2. 修改 `lib.rs` 中的 `Bq25730` 结构体和 `new()` 方法
3. 实现 `write_registers_bulk` 方法用于批量写入
4. 重构 `init()` 方法使用静态配置和批量写入
5. 更新测试用例使用新的配置模式
6. 验证所有功能保持正常

## 优势
- ✅ 消除运行时电阻检测开销
- ✅ 提高初始化速度
- ✅ 增强配置灵活性
- ✅ 更好的类型安全性
- ✅ 符合嵌入式最佳实践

## 后续工作
1. 添加配置验证逻辑
2. 实现配置保存/加载
3. 添加动态配置更新API