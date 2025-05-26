# UPS120 数据共享模块计划

**目标：** 在 `src/shared.rs` 中实现基于 `embassy-sync::pubsub` 的消息队列，用于共享设备采集数据，并定义生产者和消费者类型别名。

**设计思路：**

1. **数据结构定义：**
    * 为了按设备分类安全告警信息并聚合其他测量数据，我们将为每个设备定义两个结构体：一个用于测量数据，一个用于安全告警。
    * `Bq25730Measurements`: 包含 BQ25730 的测量数据，例如 `AdcMeasurements`。
    * `Bq25730Alerts`: 包含 BQ25730 的安全告警信息，例如 `ChargerStatus` 和 `ProchotStatus`。
    * `Bq76920Measurements`: 包含 BQ76920 的测量数据，例如 `CellVoltages`, `Temperatures`, `CoulombCounter`。
    * `Bq76920Alerts`: 包含 BQ76920 的安全告警信息，例如 `SystemStatus`。

2. **消息队列 (`pubsub`) 定义：**
    * 创建一个 `pubsub` 实例用于共享聚合的测量数据。这个 `pubsub` 将传输一个包含 `Bq25730Measurements` 和 `Bq76920Measurements` 的枚举或结构体。
    * 创建一个 `pubsub` 实例用于共享 BQ25730 的安全告警信息 (`Bq25730Alerts`)。
    * 创建一个 `pubsub` 实例用于共享 BQ76920 的安全告警信息 (`Bq76920Alerts`)。

3. **生产者和消费者类型别名：**
    * 为每个 `pubsub` 定义清晰的生产者 (`Publisher`) 和消费者 (`Subscriber`) 类型别名，方便在代码中引用和管理。

4. **非阻塞发布：**
    * 生产者在发布消息时将使用非阻塞的方式。

**实施步骤：**

1. 创建新的文件 `src/shared.rs`。
2. 在 `src/shared.rs` 中，导入所需的 `embassy-sync::pubsub` 和其他必要的依赖。
3. 定义 `Bq25730Measurements`, `Bq25730Alerts`, `Bq76920Measurements`, `Bq76920Alerts` 结构体。
4. 定义一个聚合测量数据的枚举或结构体，例如 `AllMeasurements`，包含 `Bq25730Measurements` 和 `Bq76920Measurements`。
5. 创建三个 `static` 的 `PubSub` 实例，分别用于 `AllMeasurements`, `Bq25730Alerts`, 和 `Bq76920Alerts`。
6. 定义生产者和消费者类型别名，例如 `MeasurementsPublisher`, `MeasurementsSubscriber`, `Bq25730AlertsPublisher`, `Bq25730AlertsSubscriber`, `Bq76920AlertsPublisher`, `Bq76920AlertsSubscriber`。
7. 编写示例代码，演示如何获取生产者和消费者实例，以及如何发布和订阅消息（这部分将在后续的 `code` 模式中实现）。
8. 使用 `cargo check` 和 `cargo build` 命令逐步检查代码，确保没有编译错误。

**Mermaid 图示：**

```mermaid
graph TD
    A[设备采集数据] --> B{数据分类};
    B --> C[BQ25730 测量数据];
    B --> D[BQ25730 安全告警];
    B --> E[BQ76920 测量数据];
    B --> F[BQ76920 安全告警];

    C --> G[聚合测量数据];
    E --> G;

    G --> H[测量数据 PubSub];
    D --> I[BQ25730 告警 PubSub];
    F --> J[BQ76920 告警 PubSub];

    H --> K[测量数据 消费者1];
    H --> L[测量数据 消费者2];

    I --> M[BQ25730 告警 消费者];
    J --> N[BQ76920 告警 消费者];

    O[生产者] --> H;
    O --> I;
    O --> J;
