# USB 功能模块实现计划

## 目标

在项目 `/Volumes/ExData/Projects/Ivan/ups120` 的 `/src/usb/` 目录下实现一个基于 `embassy-usb` crate 的 USB 功能模块，支持 WebUSB 通信，包含请求响应和订阅推送两种模式。重点实现 `subscribeStatus` 和 `unsubscribeStatus` 命令以及状态消息的订阅流推送。通信使用三个端点（Command、Response、Push），数据类型通过枚举表示，使用 `binrw` 进行序列化和反序列化。

## 参考实现

参考实现位于 `/Volumes/ExData/Projects/Ivan/ups120/usb-example/src/usb/` 目录，包含 `combined_endpoints.rs` 和 `mod.rs` 文件。
你必须尽可能地、全面地读取、理解并参考我提供的示例项目，它是正确无误的。

* `combined_endpoints.rs` 定义了 `UsbCommand` 枚举和 `CombinedEndpoints` 结构体，处理端点通信和命令逻辑。
* `mod.rs` 是 USB 模块入口，初始化 USB 设备和 WebUSB，并协调端点通信和数据处理。
* 状态消息数据结构使用 [`src/shared.rs`](src/shared.rs) 中的 `AllMeasurements`。
* 状态消息的发布/订阅机制使用 [`src/shared.rs`](src/shared.rs) 中的 `MEASUREMENTS_PUBSUB`。

## 详细实施计划

1. **创建 `/src/usb/` 目录和 `mod.rs` 文件：** 在您的项目 `/Volumes/ExData/Projects/Ivan/ups120/src/` 目录下创建 `usb` 目录，并在其中创建 `mod.rs` 文件。这个文件将作为您新的 USB 模块的入口。
2. **创建 `endpoints.rs` 文件：** 在 `/src/usb/` 目录下创建 `endpoints.rs` 文件，用于定义和实现端点相关的逻辑，类似于参考实现的 `combined_endpoints.rs`。
3. **定义数据枚举和结构体：**
    * 在 [`src/usb/endpoints.rs`](src/usb/endpoints.rs) 中定义一个枚举，例如 `UsbData`，用于表示 Command、Response 和 Push 数据类型。为每种数据类型定义一个枚举成员，并使用 `binrw` 的属性进行标记，以便序列化和反序列化。
    * 定义与 `subscribeStatus` 和 `unsubscribeStatus` 命令相关的枚举成员在 `UsbData` 中。
    * 状态消息的数据结构将直接使用 [`src/shared.rs`](src/shared.rs) 中的 `AllMeasurements`。
4. **在 [`src/usb/endpoints.rs`](src/usb/endpoints.rs) 中定义端点结构体：** 定义一个结构体，例如 `UsbEndpoints`，包含三个 `embassy-usb` 端点：一个用于 Command 输入 (EndpointOut)，一个用于 Response 输出 (EndpointIn)，一个用于 Push 输出 (EndpointIn)。
5. **实现端点结构体的初始化：** 在 `UsbEndpoints` 结构体中实现 `new` 函数，使用 `embassy-usb::Builder` 来配置和创建这三个端点。
6. **实现命令解析和处理：**
    * 在 `UsbEndpoints` 结构体中实现一个异步函数，例如 `parse_command`，用于从 Command 输入端点读取数据，并使用 `binrw` 反序列化为 `UsbData` 枚举中的 Command 成员。
    * 实现一个异步函数，例如 `process_command`，接收解析后的 `UsbData` 枚举作为参数，根据命令类型执行相应的逻辑。
7. **实现订阅推送逻辑：**
    * 在 `UsbEndpoints` 结构体中添加标志位，例如 `status_subscription_active`，用于指示状态消息订阅是否激活。
    * 在 `process_command` 中处理 `UsbData::SubscribeStatus` 和 `UsbData::UnsubscribeStatus` 命令，修改 `status_subscription_active` 标志位。
    * 实现一个异步函数，例如 `send_status_update`，接收 [`src/shared.rs`](src/shared.rs) 中的 `AllMeasurements` 数据作为参数，如果 `status_subscription_active` 为 true，则使用 `binrw` 序列化数据并发送到 Push 输出端点。
8. **在 [`src/usb/mod.rs`](src/usb/mod.rs) 中集成：**
    * 在 [`src/usb/mod.rs`](src/usb/mod.rs) 中定义 `usb_task` 异步函数，类似于参考实现。
    * 在 `usb_task` 中初始化 `embassy-usb` builder。
    * 创建 `UsbEndpoints` 实例。
    * 获取 [`src/shared.rs`](src/shared.rs) 中 `MEASUREMENTS_PUBSUB` 的订阅者实例。
    * 使用 `embassy_futures::select` 同时监听 Command 输入端点的读取和 `MEASUREMENTS_PUBSUB` 的新消息。
    * 在 `select` 的分支中，处理接收到的命令和状态更新，调用 `UsbEndpoints` 中相应的方法（例如，接收到 `MEASUREMENTS_PUBSUB` 的新消息时，调用 `send_status_update`）。
9. **更新 Cargo.toml：** 添加 `embassy-usb` 和 `binrw` 等必要的依赖。
10. **更新 Cargo.toml：** 添加 `embassy-usb` 和 `binrw` 等必要的依赖。
11. **在 `main.rs` 中调用 `usb_task`：** 在您的主程序中初始化 USB 驱动和 [`src/shared.rs`](src/shared.rs) 中的 pubsubs，并 spawn `usb_task`。

## Mermaid 图示

```mermaid
graph TD
    A[main.rs] --> B(usb_task);
    B --> C(embassy-usb::Builder);
    C --> D(UsbEndpoints::new);
    D --> E(Command Endpoint OUT);
    D --> F(Response Endpoint IN);
    D --> G(Push Endpoint IN);
    B --> H(embassy_futures::select);
    H --> I(Command Endpoint Read);
    H --> J(MEASUREMENTS_PUBSUB Subscriber);
    I --> K(UsbEndpoints::parse_command);
    K --> L(UsbEndpoints::process_command);
    L --> F;
    L --> G;
    J --> M(UsbEndpoints::send_status_update);
    M --> G;
