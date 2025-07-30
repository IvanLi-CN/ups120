# LED Status System Documentation

## Overview

The UPS120 system features a comprehensive LED status indication system that provides real-time visual feedback about the system's operational state. The LED is connected to GPIO25 (onboard LED on RP2040) and displays different patterns to indicate various system conditions.

## LED Status States

### State Definitions

| State | Pattern | Frequency | Description |
|-------|---------|-----------|-------------|
| **Initializing** | Medium blink | 2Hz (250ms on/off) | System startup and device initialization |
| **Fault** | Fast blink | 4Hz (125ms on/off) | Critical system fault or communication error |
| **Charging** | Slow blink | 0.5Hz (1000ms on/off) | Battery charging in progress |
| **SystemActive** | Heartbeat | 1Hz (100ms on, 900ms off) | Normal operation, all devices responding |
| **ChargingComplete** | Pattern blink | Custom pattern | Battery fully charged |
| **Discharging** | Pattern blink | Custom pattern | Battery discharging |
| **Normal** | Off | - | System idle (rarely used) |

### State Priority (Highest to Lowest)

1. **Fault** - Critical errors, communication timeouts, device failures
2. **Charging** - Active charging operations
3. **SystemActive** - Normal operation with all devices responding
4. **Normal** - System idle

## System State Determination Logic

### Initialization Phase (First 30 seconds)

- **Initializing**: Displayed when devices haven't responded yet
- **Fault**: Displayed if initialization timeout exceeded (30s)

### Communication Health Monitoring

- **Timeout Detection**: 5 seconds without data from any device triggers fault state
- **Device Tracking**: SC8815 and BQ76920 initialization and last-seen timestamps

### Status Evaluation Hierarchy

```rust
1. Check initialization timeout
2. Check communication timeouts  
3. Check BQ76920 fault conditions (OV, UV, SCD, OCD)
4. Check SC8815 charging status
5. Default to SystemActive if all devices healthy
```

## Hardware Configuration

### GPIO Configuration
- **Pin**: GPIO25 (RP2040 onboard LED)
- **Mode**: Push-pull output
- **Active Level**: High (3.3V = LED on, 0V = LED off)

### LED Control Implementation
```rust
// LED initialization
let led_pin = Output::new(p.PIN_25, Level::Low);

// LED control patterns
match status {
    LedStatus::Initializing => {
        // 2Hz blink: 250ms on, 250ms off
        if now.duration_since(last_update) >= Duration::from_millis(250) {
            led.toggle();
        }
    }
    LedStatus::SystemActive => {
        // 1Hz heartbeat: 100ms on, 900ms off
        let cycle_time = now.duration_since(last_update);
        if cycle_time >= Duration::from_millis(1000) {
            led.set_high();  // Start new cycle
        } else if cycle_time >= Duration::from_millis(100) {
            led.set_low();   // Turn off after 100ms
        }
    }
    // ... other patterns
}
```

## Startup Sequence

### LED Test Phase (First 600ms)
1. **LED Test**: 3 rapid blinks (100ms on/off) to verify LED functionality
2. **Purpose**: Confirms LED hardware and task operation
3. **Log**: "Testing LED functionality..." → "LED test completed"

### Initialization Phase (600ms - 30s)
1. **Initial State**: Forced to `Initializing` status
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

#### Normal Startup
```
0.004474 [INFO] LED status task started
0.004492 [INFO] Testing LED functionality...
0.604697 [INFO] LED test completed
0.604798 [INFO] SC8815 device initialized and responding  
0.604884 [INFO] BQ76920 device initialized and responding
0.604993 [INFO] LED status changed to: SystemActive
```

#### Fault Condition
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

## Integration with System Components

### Data Sources
- **SC8815**: Charging status, measurements, alerts
- **BQ76920**: Battery protection status, cell voltages, alerts  
- **System**: Device initialization status, communication health

### Task Communication
- **PubSub System**: Non-blocking message passing between tasks
- **Subscribers**: LED task subscribes to alerts and measurements
- **Update Rate**: 10ms LED task cycle for responsive pattern updates

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
