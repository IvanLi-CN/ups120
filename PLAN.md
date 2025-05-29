# 在 `src/shared.rs` 中添加 INA226 PubSub 相关代码的计划

## 目标

在项目中集成 INA226 测量数据的 PubSub 机制，以便在不同模块间共享 INA226 的测量数据。

## 计划步骤

1. **定义 INA226 测量数据类型 (`src/data_types.rs`)：**
    * 在 `src/data_types.rs` 中创建一个新的结构体 `Ina226Measurements`，用于存储 INA226 的电压、电流和功率测量数据。
    * 为该结构体添加必要的派生宏，如 `Debug`, `Copy`, `Clone`, `PartialEq`, `defmt::Format`, `binrw::binrw`。
    * 包含电压、电流和功率的字段，可能会使用 `uom` 类型来表示单位。

2. **更新 `AllMeasurements` 结构体 (`src/data_types.rs`)：**
    * 在 `src/data_types.rs` 的 `AllMeasurements` 结构体中添加一个 `Ina226Measurements` 类型的字段。
    * 更新 `AllMeasurements` 的 `Format` 实现，以包含新的 INA226 数据。

3. **添加 INA226 PubSub 通道 (`src/shared.rs`)：**
    * 在 `src/shared.rs` 中定义 INA226 PubSub 的深度和读者数量常量，类似于现有的常量。
    * 声明一个 `static StaticCell` 用于 INA226 的 `PubSubChannel`，使用新的 `Ina226Measurements` 数据类型。

4. **添加 INA226 PubSub 类型别名 (`src/shared.rs`)：**
    * 在 `src/shared.rs` 中定义 `Ina226MeasurementsPublisher` 和 `Ina226MeasurementsSubscriber` 类型别名，类似于现有的别名。

5. **更新 `init_pubsubs` 函数 (`src/shared.rs`)：**
    * 在 `src/shared.rs` 的 `init_pubsubs` 函数中初始化 INA226 的 `PubSubChannel`。
    * 将 INA226 的发布者和订阅者添加到 `init_pubsubs` 函数的返回值元组中。
    * 相应地更新函数的签名和返回类型。

## 计划图示

```mermaid
graph TD
    A[src/data_types.rs] --> B{添加 Ina226Measurements 结构体};
    A --> C{更新 AllMeasurements 结构体};
    A --> D{更新 AllMeasurements Format 实现};
    E[src/shared.rs] --> F{添加 INA226 PubSub 常量};
    E --> G{添加 INA226 PubSub StaticCell};
    E --> H{添加 INA226 PubSub 类型别名};
    E --> I{更新 init_pubsubs 函数};
    B --> C;
    C --> D;
    F --> G;
    G --> I;
    H --> I;
