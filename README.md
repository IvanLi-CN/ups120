# UPS120 RP2040 Firmware

Intelligent UPS (Uninterruptible Power Supply) control system based on RP2040 microcontroller, developed with Embassy async framework.

## 🚀 Project Overview

UPS120 is a complete UPS control solution that integrates battery management, charging control, power monitoring, and USB communication capabilities. This project has been successfully migrated from STM32G431CB platform to RP2040, maintaining all core functionalities while optimizing performance.

### Core Features

- **🔋 Battery Management System (BQ76920)**
  - 5-cell lithium battery monitoring and protection
  - Overvoltage, undervoltage, and overcurrent protection
  - Intelligent battery balancing
  - Temperature monitoring

- **⚡ Charging Control (SC8815)**
  - Intelligent charging management
  - Multiple charging modes
  - Charging status monitoring
  - Safety protection mechanisms

- **📊 Power Monitoring (INA226)**
  - Input power monitoring
  - Real-time power measurement
  - Voltage and current detection

- **💻 USB Communication**
  - WebUSB support
  - Real-time data streaming
  - Command control interface
  - Direct browser access

- **💡 Status Indication**
  - LED status display
  - Charging/discharging indication
  - Fault alarms

## 🛠 Hardware Requirements

### Main Controller

- **Raspberry Pi Pico** or other RP2040 development boards
- **Debugger**: Raspberry Pi Debug Probe or Picoprobe

### Peripheral Devices

- **BQ76920**: Battery management IC (I2C address: 0x18)
- **SC8815**: Charging management IC (I2C address: 0x6A)
- **INA226**: Power monitoring IC (I2C address: 0x40)

### Pin Configuration

```text
GP0  - I2C0 SDA (all I2C devices)
GP1  - I2C0 SCL (all I2C devices)
GP3  - BQ76920 discharge control
GP4  - BQ76920 charge control
GP25 - Status LED (onboard)
```

## 📦 Software Requirements

### Development Environment

```bash
# Install Rust target
rustup target add thumbv6m-none-eabi

# Install probe-rs for flashing and debugging
cargo install probe-rs --features cli

# Install Node.js dependencies (for code quality tools)
npm install
```

### Development Tools

- **Rust 2024 Edition**
- **Embassy async framework**
- **probe-rs** (flashing and debugging)
- **lefthook** (Git hooks)
- **commitlint** (commit conventions)

## 🚀 Quick Start

### 1. Clone Project

```bash
git clone <repository-url>
cd ups120
```

### 2. Install Dependencies

```bash
# Install Rust dependencies
cargo build

# Install Node.js dependencies
npm install

# Install Git hooks
npm run install-hooks
```

### 3. Build and Flash

```bash
# Build project
make build

# Flash to device
make run

# Or use cargo directly
cargo run
```

### 4. Debug

```bash
# Attach debugger
make attach

# Reset and attach
make reset-attach

# Check firmware size
make size
```

## 📊 System Performance

### Firmware Metrics

- **Firmware Size**: ~118 KB (120,580 bytes)
- **Flash Utilization**: 5.75% (1.88MB remaining)
- **RAM Utilization**: 8.68% (240KB remaining)
- **Update Frequency**: 1-second cycle

### Task Architecture

```text
┌─────────────────┐    ┌─────────────────┐
│   BQ76920 Task  │    │   SC8815 Task   │
│ (Battery Mgmt)  │    │ (Charge Ctrl)   │
└─────────────────┘    └─────────────────┘
         │                       │
         └───────┬───────────────┘
                 │
    ┌─────────────────┐    ┌─────────────────┐
    │   INA226 Task   │    │   LED Task      │
    │ (Power Monitor) │    │ (Status Ind.)   │
    └─────────────────┘    └─────────────────┘
                 │                 │
                 └─────┬───────────┘
                       │
              ┌─────────────────┐
              │    USB Task     │
              │ (Data Comm.)    │
              └─────────────────┘
```

## 🔧 Development Tools

### Makefile Commands

```bash
# Build related
make build          # Build debug version
make build-release  # Build release version
make clean          # Clean build files

# Run and debug
make run            # Build and flash
make attach         # Attach debugger
make reset          # Reset device
make size           # Check firmware size

# Code quality
make dev-check      # Run fmt + clippy + check
make fmt            # Format code
make clippy         # Code linting
```

### Git Hooks

Project configured with strict code quality checks:

- **pre-commit**: Automatically runs `fmt`, `clippy`, `check`
- **commit-msg**: Enforces conventional commits standard
- **English only**: Prohibits non-English commit messages

## 📡 USB Communication Protocol

### WebUSB Interface

System supports direct browser access through WebUSB:

```javascript
// Connect to device
const device = await navigator.usb.requestDevice({
  filters: [{ vendorId: 0x16c0, productId: 0x27dd }]
});

// Subscribe to real-time data
const command = { SubscribeStatus: null };
await device.transferOut(1, new TextEncoder().encode(JSON.stringify(command)));
```

### Data Format

```json
{
  "ina226_voltage_mv": 12000,
  "ina226_current_ma": 1500,
  "ina226_power_mw": 18000,
  "sc8815_adc_vbus_mv": 12100,
  "sc8815_adc_vbat_mv": 11800,
  "bq76920_cell_voltages": [3800, 3810, 3805, 3795, 3800],
  "bq76920_pack_voltage_mv": 19010,
  "bq76920_temperature_c": 25.5
}
```

## 🏗 Project Structure

```text
ups120/
├── src/
│   ├── main.rs              # Main program entry
│   ├── data_types.rs        # Data structure definitions
│   ├── shared.rs            # Shared resources and PubSub
│   ├── led_status_task.rs   # LED status task
│   ├── bq76920_task.rs      # Battery management task
│   ├── charger_task.rs      # Charging control task
│   ├── ina226_task.rs       # Power monitoring task
│   └── usb/                 # USB communication module
│       ├── mod.rs           # USB main module
│       └── endpoints.rs     # USB endpoint handling
├── bq76920/                 # BQ76920 driver library
├── sc8815/                  # SC8815 driver library
├── Cargo.toml               # Project configuration
├── Makefile                 # Build tools
├── memory.x                 # Memory layout
├── build.rs                 # Build script
├── lefthook.yml             # Git hooks configuration
├── commitlint.config.cjs    # Commit convention config
├── package.json             # Node.js dependencies
└── rustfmt.toml             # Code formatting config
```

## 🔍 Troubleshooting

### Common Issues

1. **Compilation Errors**

   ```bash
   # Ensure target is installed
   rustup target add thumbv6m-none-eabi

   # Clean and rebuild
   make clean && make build
   ```

2. **Flashing Failures**

   ```bash
   # Check device connection
   probe-rs list

   # Reset device
   make reset
   ```

3. **I2C Communication Issues**
   - Check external pull-up resistors (4.7kΩ)
   - Verify correct device addresses
   - Check pin connections

### Debug Output

Use `defmt` and RTT to view debug information:

```bash
# Attach and view logs
make attach
```

## 🤝 Contributing

### Code Standards

- Follow official Rust code style
- Use `cargo fmt` to format code
- Pass `cargo clippy` checks
- Follow conventional commits for commit messages

### Contribution Workflow

```bash
# 1. Create feature branch
git checkout -b feature/new-feature

# 2. Develop and test
make dev-check

# 3. Commit (hooks will run automatically)
git commit -m "feat: add new feature"

# 4. Push and create PR
git push origin feature/new-feature
```

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## 🙏 Acknowledgments

- [Embassy](https://github.com/embassy-rs/embassy) - Async embedded framework
- [probe-rs](https://github.com/probe-rs/probe-rs) - Embedded debugging tools
- [defmt](https://github.com/knurling-rs/defmt) - Efficient logging framework

---

## 🚀 UPS120 - Professional RP2040 UPS Control System
