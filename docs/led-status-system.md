# UPS120 LED Status System Documentation

## Overview

The UPS120 system features a comprehensive LED status indication system that provides real-time visual feedback about the system's operational state. The LED is connected to GPIO25 (onboard LED on RP2040) and displays different patterns to indicate various system conditions.

## System States

### Global State Overview

| State Name | Battery Status | Charger Status | Backup Power Module | LED Pattern | Description |
|------------|----------------|----------------|---------------------|-------------|-------------|
| **Initialization** | Unknown | Unknown | Unknown | Medium blink (2Hz) | System startup and device initialization |
| **Fault** | Any state | Any state | Any state | Fast blink (4Hz) | Any component fault or communication error |
| **Charging with Mains** | Charging | Active | Standby | Double flash every 3s | Mains power normal, battery charging, load powered by mains |
| **Mains without Charging** | Full/Standby | Float/Standby | Standby | Triple flash every 3s | Mains power normal, battery full, load powered by mains |
| **Backup Power Output** | Discharging | Stopped | Active | Custom pattern | Mains interrupted, battery discharging, load powered by backup |

### State Priority (Highest to Lowest)

1. **Fault** - Any component fault or communication timeout triggers immediately
   - Battery protection triggered (OV/UV/OCD/SCD)
   - Charger anomaly (temperature/voltage abnormal)
   - Communication timeout (device non-responsive)
   - Backup power module fault
2. **Backup Power Output** - Mains power interrupted, battery supplying load
3. **Charging with Mains** - Mains power normal, battery needs charging
4. **Mains without Charging** - Mains power normal, battery fully charged
5. **Initialization** - System startup phase

## Component Status Details

### Battery Management (BQ76920)

- **Normal**: Voltage within range, no protection triggered
- **Charging**: Accepting charge current, voltage rising
- **Full**: Reached charge termination voltage
- **Discharging**: Supplying current to load
- **Fault**: Protection triggered (OV/UV/SCD/OCD)
- **Standby**: No charge/discharge activity
- **Unknown**: Initialization phase or communication lost

### Charger Controller (SC8815)

- **Active**: Actively charging, outputting current
- **Float/Standby**: Maintaining voltage, trickle current
- **Stopped**: No output, charger disabled
- **Fault**: Temperature/voltage abnormal
- **Normal**: Device responding, parameters in range
- **Unknown**: Initialization phase or communication lost

### Backup Power Module

- **Active**: Actively outputting stable voltage to load
- **Standby**: Ready state, no output demand
- **Normal**: Module operating normally, voltage stable
- **Fault**: Output voltage abnormal, overload protection
- **Unknown**: Initialization phase or status unclear

## Hardware Configuration

### GPIO Configuration

- **Pin**: GPIO25 (RP2040 onboard LED)
- **Mode**: Push-pull output
- **Active Level**: High (3.3V = LED on, 0V = LED off)

### LED Pattern Implementation

```rust
// LED initialization
let led_pin = Output::new(p.PIN_25, Level::Low);

// LED control patterns
match status {
    LedStatus::Initialization => {
        // 2Hz blink: 250ms on, 250ms off
        if now.duration_since(last_update) >= Duration::from_millis(250) {
            led.toggle();
        }
    }
    LedStatus::ChargingWithMains => {
        // Double flash every 3 seconds
        let cycle_time = now.duration_since(last_update).as_millis() % 3000;
        match cycle_time {
            0..=100 => led.set_high(),      // First flash
            100..=200 => led.set_low(),
            200..=300 => led.set_high(),    // Second flash
            300..=3000 => led.set_low(),    // Long off period
            _ => {}
        }
    }
    LedStatus::MainsWithoutCharging => {
        // Triple flash every 3 seconds
        let cycle_time = now.duration_since(last_update).as_millis() % 3000;
        match cycle_time {
            0..=100 => led.set_high(),      // First flash
            100..=200 => led.set_low(),
            200..=300 => led.set_high(),    // Second flash
            300..=400 => led.set_low(),
            400..=500 => led.set_high(),    // Third flash
            500..=3000 => led.set_low(),    // Long off period
            _ => {}
        }
    }
    LedStatus::Fault => {
        // 4Hz fast blink: 125ms on, 125ms off
        if now.duration_since(last_update) >= Duration::from_millis(125) {
            led.toggle();
        }
    }
    LedStatus::BackupPowerOutput => {
        // Custom pattern for backup power mode
        // Implementation depends on specific requirements
    }
}
```

## State Transition Logic

### Typical State Transition Scenarios

**Normal Startup Flow:**

```
Initialization → (Detect mains and battery status) → Charging with Mains/Mains without Charging
```

**Mains Power Interruption:**

```
Charging with Mains/Mains without Charging → Backup Power Output → (Mains restored) → Charging with Mains/Mains without Charging
```

**Fault Handling:**

```
Any State → Fault → (Fault cleared) → Initialization → Normal State
```

**Charging State Transitions:**

```
Charging with Mains → (Battery full) → Mains without Charging
Mains without Charging → (Battery level drops) → Charging with Mains
```

## System Integration

### Communication Health Monitoring

- **Timeout Detection**: 5 seconds without data from any device triggers fault state
- **Device Tracking**: SC8815 and BQ76920 initialization and last-seen timestamps
- **Initialization Timeout**: 30 seconds maximum for device initialization

### Data Sources

- **SC8815**: Charging status, measurements, alerts
- **BQ76920**: Battery protection status, cell voltages, alerts
- **System**: Device initialization status, communication health

### Task Communication

- **PubSub System**: Non-blocking message passing between tasks
- **Subscribers**: LED task subscribes to alerts and measurements
- **Update Rate**: 10ms LED task cycle for responsive pattern updates

## Startup Sequence

### LED Test Phase (First 600ms)

1. **LED Test**: 3 rapid blinks (100ms on/off) to verify LED functionality
2. **Purpose**: Confirms LED hardware and task operation
3. **Log**: "Testing LED functionality..." → "LED test completed"

### Initialization Phase (600ms - 30s)

1. **Initial State**: Forced to `Initialization` status
2. **Pattern**: 2Hz medium blink (gentle initialization indicator)
3. **Transition**: Changes to appropriate status once devices respond

### Normal Operation (After device initialization)

1. **Status Evaluation**: Continuous monitoring of all system components
2. **Dynamic Updates**: LED pattern changes based on real-time system state
3. **Debug Logging**: Status updates every 5 seconds for troubleshooting

## Troubleshooting

### LED Never Turns On

1. **Check Hardware**: Verify GPIO25 connection and LED polarity
2. **Check Task**: Look for "LED status task started" in logs
3. **Check Test**: Look for "Testing LED functionality..." message
4. **Check Power**: Ensure RP2040 has proper 3.3V supply

### LED Stuck in One Pattern

1. **Check Logs**: Monitor debug output every 5 seconds
2. **Check Device Status**: Verify SC8815 and BQ76920 initialization
3. **Check Communication**: Look for timeout or error messages

### Expected Behavior Examples

**Normal Startup:**

```
0.004474 [INFO] LED status task started
0.004492 [INFO] Testing LED functionality...
0.604697 [INFO] LED test completed
0.604798 [INFO] SC8815 device initialized and responding
0.604884 [INFO] BQ76920 device initialized and responding
0.604993 [INFO] LED status changed to: ChargingWithMains
```

**Fault Condition:**

```
[ERROR] SC8815 communication timeout
[INFO] LED status changed to: Fault
```

## Debug Information

### Status Logging

- **Frequency**: Every 5 seconds
- **Format**: `LED Debug - Status: {status}, SC8815_init: {bool}, BQ76920_init: {bool}`
- **Purpose**: Real-time system health monitoring

### State Change Logging

- **Trigger**: Whenever LED status changes
- **Format**: `LED status changed to: {new_status}`
- **Purpose**: Track system state transitions

## Future Enhancements

### Potential Improvements

1. **RGB LED Support**: Multi-color status indication
2. **Brightness Control**: PWM-based intensity adjustment
3. **Custom Patterns**: User-configurable blink sequences
4. **Remote Control**: USB/network-based LED control
5. **Pattern Persistence**: Remember last state across resets

### Configuration Options

1. **Pattern Timing**: Adjustable blink frequencies
2. **Priority Levels**: Customizable state hierarchy
3. **Timeout Values**: Configurable communication timeouts
4. **Debug Levels**: Selectable logging verbosity
