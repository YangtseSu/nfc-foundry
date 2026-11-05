# Backlog: card emulation research

Card emulation is a future direction, especially for mobile Linux devices. It
is **not** part of the current phases and is not exposed by the GUI or CLI.

Research questions:

- Which emulated target types are needed (Type 2, Type 4, ISO14443-4, others)?
- Which PN53x/PN71xx firmware and Linux targets support target mode reliably?
- How should emulation state be isolated from initiator card workflows?
- What validation is required against real readers and systems?

Keep target-mode and raw-frame abstractions in the transport design, but do not
add emulation commands or UI until this research produces an accepted spec.
