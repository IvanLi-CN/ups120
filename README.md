# Ivan's UPS Firmware

**This project is currently under development.**

This project contains the firmware for a digitally controlled Uninterruptible Power Supply (UPS) with a power capacity of 120W.

## Hardware Connection

Here is a brief overview of the hardware connections:

* **Battery:** Connect a compatible battery to the designated battery connector.
* **Power Input:** Connect a power source (e.g., AC adapter) to the power input connector.
* **Load Output:** Connect the load to the load output connector.
* **Communication:** Connect the communication interface (e.g., I2C, UART) to the designated pins for communication with a host device.

## Hardware Information

* **UPS Capacity:** 120W
* **Battery:** 5S LiFePO4 (Lithium Iron Phosphate) battery pack
* **Battery Management IC:** BQ76920
* **Battery Charger IC:** BQ25730
* **Microcontroller:** STM32G031

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
