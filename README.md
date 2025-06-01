# Ivan's UPS Firmware

**This project is currently under development.**

This project contains the firmware for a digitally controlled Uninterruptible Power Supply (UPS) with a power capacity of 120W.

For a detailed description of the project's MVP (Minimum Viable Product) business workflow, including device initialization, data acquisition, and control logic, please see the [WORKFLOW.md](WORKFLOW.md) file.

## Hardware Connection

Here is a brief overview of the hardware connections:

* **Battery:** Connect a compatible battery to the designated battery connector.
* **Power Input:** Connect a power source (e.g., AC adapter) to the power input connector.
* **Load Output:** Connect the load to the load output connector.
* **Communication:** Connect the communication interface (e.g., I2C, UART) to the designated pins for communication with a host device.

## Hardware Information

* **UPS Capacity:** 120W
* **Battery:** 5S LiFePO4 (Lithium Iron Phosphate) battery pack (e.g., nominal 16V, full charge ~18.25V)
* **Battery Management IC:** BQ76920 (AFE for 3-5 series Li-Ion/LiFePO4 cells)
* **Battery Charger IC:** BQ25730 (NVDC Buck-Boost Charger for 1-5 series cells, configured for 5S LiFePO4, ~18V charge voltage)
* **Current/Voltage Sensor:** INA226 (High-Side/Low-Side I2C Current and Power Monitor, e.g., 16V bus voltage range)
* **Microcontroller:** STM32G031 (e.g., STM32G031F8P6 Arm Cortex-M0+)

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
