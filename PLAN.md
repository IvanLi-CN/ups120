# 计划

1.  **添加 `ina226` crate:** 修改 `Cargo.toml` 文件，添加 `ina226 = { version = "0.3.0", features = ["async"] }` 作为依赖项。
2.  **确定 I2C 总线:** 确定代码需要在哪个 I2C 总线上与 INA226 芯片通信。这通常涉及到硬件配置，例如使用哪个 I2C 外设（例如 `I2C1`，`I2C2` 等）。
3.  **初始化 I2C 总线:** 编写代码初始化 I2C 总线，并配置 I2C 外设。
4.  **创建 INA226 实例:** 使用 `ina226` crate 创建 INA226 芯片的实例，指定 I2C 地址 0x40。
5.  **读取电流和电压:** 调用 INA226 实例的方法，读取电流和电压数据。
6.  **处理数据:** 对读取到的数据进行处理，例如单位转换或校准。检流电阻目前是 10 mohm。
7.  **错误处理:** 添加错误处理代码，处理 I2C 通信错误或 INA226 读取错误。
8.  **测试代码:** 编写测试代码，验证读取到的电流和电压数据是否正确。

## Mermaid 图表

```mermaid
sequenceDiagram
    participant User
    participant Architect
    participant Cargo.toml
    participant I2C Bus
    participant INA226

    User->>Architect: 提供任务：读取 INA226 电流电压信息
    Architect->>Cargo.toml: 检查 ina226 crate 是否存在
    alt ina226 crate 不存在
        Architect->>Cargo.toml: 添加 ina226 crate
    end
    Architect->>User: 询问目标平台，I2C 总线，错误处理方式，数据处理方式
    User->>Architect: 提供目标平台，I2C 总线，错误处理方式，数据处理方式
    Architect->>I2C Bus: 初始化 I2C 总线
    Architect->>INA226: 创建 INA226 实例 (地址 0x40)
    loop 读取数据
        Architect->>INA226: 读取电流和电压
        INA226-->>Architect: 返回电流和电压数据
        Architect->>Architect: 处理数据
        Architect->>User: 返回处理后的数据
    end