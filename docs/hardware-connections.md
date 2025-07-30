# UPS120 RP2040 Hardware Connections

This document provides detailed GPIO pin assignments and hardware connection specifications for the UPS120 RP2040 firmware.

## 📋 Pin Assignment Overview

| GPIO Pin | Function | Direction | Pull | Connected Device | Description |
|----------|----------|-----------|------|------------------|-------------|
| GP0 | I2C0 SDA | Bidirectional | External | All I2C devices | I2C data line |
| GP1 | I2C0 SCL | Output | External | All I2C devices | I2C clock line |
| GP2 | PSTOP Control | Output | None | SC8815 | Charging enable/disable |
| GP3 | Discharge Control | Input | Pull-up | BQ76920 | Discharge control input |
| GP4 | Charge Control | Input | Pull-up | BQ76920 | Charge control input |
| GP25 | Status LED | Output | None | Onboard LED | System status indication |

## 🔌 I2C Bus Configuration

### I2C0 Bus (GP0/GP1)
- **Frequency**: 100 kHz
- **Pull-up resistors**: 4.7kΩ external resistors to 3.3V (required)
- **Voltage level**: 3.3V logic

### Connected I2C Devices

| Device | I2C Address (7-bit) | I2C Address (8-bit) | Function |
|--------|---------------------|---------------------|----------|
| BQ76920 | 0x08 | 0x10/0x11 | Battery management IC |
| SC8815 | 0x6A | 0xD4/0xD5 | Charging management IC |
| INA226 | 0x40 | 0x80/0x81 | Power monitoring IC |

## 🔋 BQ76920 Battery Management IC

### Pin Connections
```text
RP2040 Pin    BQ76920 Pin    Function
----------    -----------    --------
GP0 (SDA)  -> SDA           I2C data
GP1 (SCL)  -> SCL           I2C clock
GP3        <- DSG           Discharge control output
GP4        <- CHG           Charge control output
3.3V       -> VCC           Power supply
GND        -> VSS           Ground
```

### Control Logic
- **GP3 (Discharge Control)**:
  - `LOW` = Discharge enabled (BQ76920 allows discharge)
  - `HIGH` = Discharge disabled (BQ76920 blocks discharge)
  - Internal pull-up enabled on RP2040

- **GP4 (Charge Control)**:
  - `LOW` = Charge allowed (BQ76920 allows charging)
  - `HIGH` = Charge disabled (BQ76920 blocks charging)
  - Internal pull-up enabled on RP2040

### Safety Features
- Overvoltage protection: 3.6V per cell
- Undervoltage protection: 2.5V per cell
- Overcurrent protection: 10A discharge current
- Temperature monitoring via NTC

## ⚡ SC8815 Charging Management IC

### Pin Connections
```text
RP2040 Pin    SC8815 Pin     Function
----------    ----------     --------
GP0 (SDA)  -> SDA            I2C data
GP1 (SCL)  -> SCL            I2C clock
GP2        -> PSTOP          Power stage control
3.3V       -> VCC            Power supply
GND        -> GND            Ground
```

### PSTOP Control (GP2)
- **Function**: Power stage enable/disable control
- **Logic**:
  - `HIGH` = Standby mode (power blocks disabled, I2C active) - SAFE
  - `LOW` = Active mode (power blocks enabled) - OPERATIONAL
- **Safety**: Always start in HIGH state for safe configuration

### Configuration
- **Operating mode**: Charging mode (not OTG)
- **Current limits**: 500mA input/output
- **Switching frequency**: 450kHz
- **Sense resistors**: 5mΩ (RS1, RS2)
- **VINREG voltage**: 11.5V

## 📊 INA226 Power Monitoring IC

### Pin Connections
```text
RP2040 Pin    INA226 Pin     Function
----------    ----------     --------
GP0 (SDA)  -> SDA            I2C data
GP1 (SCL)  -> SCL            I2C clock
3.3V       -> VCC            Power supply
GND        -> GND            Ground
VIN+       -> Input+         Positive input voltage sense
VIN-       -> Input-         Negative input voltage sense
```

### Configuration
- **Shunt resistor**: 0.01Ω (10mΩ)
- **Maximum current**: 10A
- **Voltage range**: 0-36V
- **Calibration**: Configured for 0.01Ω shunt, 10A max

## 💡 Status LED (GP25)

### Configuration
- **Pin**: GP25 (onboard LED on Raspberry Pi Pico)
- **Logic**: Active HIGH (LED on when pin is HIGH)
- **Function**: System status indication

### LED States
- **Solid ON**: System operational, all sensors responding
- **Slow blink (1Hz)**: Normal operation, data streaming
- **Fast blink (5Hz)**: Warning condition or sensor error
- **OFF**: System not initialized or critical error

## 🔧 Hardware Setup Requirements

### Power Supply
- **RP2040 VCC**: 3.3V (via USB or external regulator)
- **All I2C devices**: 3.3V logic level
- **Current consumption**: ~50mA typical

### I2C Pull-up Resistors
```text
3.3V ----[4.7kΩ]---- GP0 (SDA)
3.3V ----[4.7kΩ]---- GP1 (SCL)
```
**Critical**: External pull-up resistors are required for reliable I2C communication.

### PCB Layout Considerations
- Keep I2C traces short and equal length
- Place pull-up resistors close to RP2040
- Use ground plane for noise reduction
- Separate analog and digital grounds if possible

## 🔍 Debugging and Testing

### I2C Bus Testing
```bash
# Check I2C device detection
make attach
# Look for initialization messages:
# "BQ76920 configuration applied successfully"
# "SC8815 configuration applied successfully"  
# "INA226 calibration successful"
```

### GPIO Testing
- **LED test**: Should blink during normal operation
- **Control inputs**: Monitor GP3/GP4 for BQ76920 control signals
- **PSTOP control**: GP2 should start HIGH, go LOW after configuration

### Common Issues
1. **I2C communication failure**:
   - Check pull-up resistors (4.7kΩ to 3.3V)
   - Verify device addresses
   - Check wiring connections

2. **BQ76920 not responding**:
   - Verify GP3/GP4 connections
   - Check power supply voltage
   - Ensure proper grounding

3. **SC8815 configuration failure**:
   - Check PSTOP pin (GP2) connection
   - Verify I2C address (0x6A)
   - Ensure device is powered

## 📐 Schematic Reference

### Minimal Connection Diagram
```text
                    RP2040 Pico
                   ┌─────────────┐
                   │             │
    4.7kΩ to 3.3V ─┤GP0 (SDA)    │
    4.7kΩ to 3.3V ─┤GP1 (SCL)    │
                   │GP2          ├─ SC8815 PSTOP
                   │GP3          ├─ BQ76920 DSG
                   │GP4          ├─ BQ76920 CHG
                   │             │
                   │GP25 (LED)   ├─ Status LED
                   │             │
                   └─────────────┘
                          │
                    ┌─────┴─────┐
                    │ I2C Bus   │
                    │ (3.3V)    │
                    └─┬─────┬─┬─┘
                      │     │ │
                 BQ76920  SC8815  INA226
                 (0x08)   (0x6A)  (0x40)
```

## ⚠️ Safety Warnings

1. **Power sequencing**: Always configure devices before enabling power stages
2. **PSTOP control**: Keep GP2 HIGH during configuration to prevent damage
3. **Current limits**: Verify sense resistor values match firmware configuration
4. **Voltage levels**: Ensure all devices operate at 3.3V logic levels
5. **Grounding**: Maintain proper ground connections for all devices

---

**Note**: This document reflects the current firmware implementation. Always verify pin assignments match your specific hardware design before connecting devices.
