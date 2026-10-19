# CAN FD hardware validation

Status: **pending**. No CANable 2.0-compatible adapter or CAN FD peer was available in the development environment. Software tests with an injected serial stream cannot establish successful CAN bus delivery.

Record the adapter model, firmware repository and exact revision/version, host OS, serial port, serial baud rate, CAN FD peer, nominal bitrate, and data bitrate before testing. Use a bus with correct termination and a peer that can report received frames and errors.

At 500 kbit/s nominal and 2 Mbit/s data phase, verify both transmit and receive for:

- Standard and extended IDs, each with BRS off and on.
- Every FD DLC from 0 through 15, including 64 bytes.
- Classic data and RTR frames mixed with FD traffic.
- Invalid FD lengths and unsupported data rates rejected before serial commands.
- Disconnect during a pending send and receive; confirm waiting tasks terminate.

Record the observed result for each case and any adapter status/error output. An adapter serial write or command ACK does not prove that a CAN peer acknowledged a frame. Keep this file marked pending until the physical peer results and firmware identity are documented.
