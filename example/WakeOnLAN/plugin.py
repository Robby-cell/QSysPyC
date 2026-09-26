from qsys import Control, ControlHandle, QSysPlugin, UdpSocket, qsys


class WakeOnLan(QSysPlugin):
    version = "1.0.0"
    author = "AV Engineering"
    description = "Sends Wake-on-LAN magic packets over UDP broadcast"

    # 1. Built-in Q-SYS IPAddress control (pre-configured with pins="Both")
    IPAddress = Control.IPAddress(default="255.255.255.255")

    # 2. Built-in Q-SYS MACAddress control (pre-configured with pins="Both")
    MACAddress = Control.MACAddress(default="00:11:22:33:44:55")

    # 3. Dedicated UI push button (pins="None" keeps it on the UI panel only)
    SendButton = Control.Button(name="Send", type="Trigger", pins="None")

    # 4. Dedicated schematic input pin (pins="Input" exposes the wiring pin on the block)
    TriggerPin = Control.Button(name="Trigger", type="Trigger", pins="Input")

    # 5. Status feedback output pin
    Status = Control.Text(name="Status", default="Ready", pins="Output")

    def send_packet(self):
        # Native Q-SYS socket API with full autocomplete & typing!
        sock = UdpSocket.New()
        sock.Open()

        # Binary magic packet payload
        packet = "\xff" * 6 + "\x00\x11\x22\x33\x44\x55" * 16

        # Automatically compiled to Lua colon method: sock:Send(...)
        sock.Send(self.IPAddress.String, 9, packet)
        sock.Close()

        self.Status.String = "Packet sent"

    @SendButton.EventHandler
    def on_ui_press(self, ctl: ControlHandle):
        """Fires when clicked from the UI panel."""
        self.send_packet()

    @TriggerPin.EventHandler
    def on_pin_pulse(self, ctl: ControlHandle):
        """Fires when pulsed via external schematic wire."""
        self.send_packet()
