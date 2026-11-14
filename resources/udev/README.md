# udev rules

Do not install broad `MODE="0666"` rules. `70-nfc-foundry.rules` grants the
logged-in user access to the ACR122U PICC interface only.

## ACR122U

```sh
sudo install -m 0644 resources/udev/70-nfc-foundry.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules
sudo udevadm trigger --subsystem-match=usb
```

The rule matches `072F:2200` and uses `uaccess`, so no group membership is
required. Replug the reader after installing the rule.

`pcscd` claims the same USB interface through libusb. Stop it before using the
`acr122_usb` backend:

```sh
sudo systemctl stop pcscd
```

The driver reports a busy interface with an instruction to stop `pcscd`. To use
the `acr122_pcsc` backend instead, keep `pcscd` running (its socket activates
it on demand) and select the exact named reader:

```sh
nfctl list
nfctl --device 'acr122_pcsc:ACS ACR122U 00 00' scan
```

## PN532 UART

The serial adapter is accessed through `/dev/ttyUSB0` or the exact port
reported by `nfctl list`. Add the user to `uucp` on Arch Linux (or `dialout` on
other distributions) when the port is not accessible:

```sh
sudo usermod -aG uucp "$USER"
```

Log out and back in after changing group membership. Do not add a udev rule
that grants access to every serial port.
