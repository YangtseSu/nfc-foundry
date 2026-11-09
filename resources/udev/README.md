# udev rules

The PN532 UART and ACR122U USB rules are added with their drivers in phases
02 and 05. Do not install broad `MODE="0666"` rules.

Expected scope:

- ACR122U USB vendor/product IDs only, with a dedicated `uaccess` rule.
- PN532 UART adapters by stable serial/interface path where possible.
- Document the `dialout`/`uucp` group requirement and the exact connstring.
