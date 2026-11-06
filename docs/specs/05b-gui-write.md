# Phase 05b spec: GUI write workflows

Date: 2026-11-07

## Goal

Connect the existing Type 2 write services to the MyGo GUI with a confirmation
surface and asynchronous task reporting. Protocol framing, backups, read-back
verification, and card-change checks remain in `internal/type2` and
`internal/workflow`.

## Scope

GUI writes are limited to:

- one user-memory page at a time;
- one NDEF Text or URI message.

Protected pages, configuration, PWD/PACK, dynamic lock, static lock, and pages
0-3 remain CLI-only. Classic GUI writes are out of scope.

## Application service

Add async task entry points that wrap the synchronous write methods:

```go
type Type2WriteStartRequestDTO struct {
    Pages    []Type2PageWriteDTO
    Password string
}

type Type2NDEFWriteStartRequestDTO struct {
    Mode     string // "text" or "uri"
    Text     string
    URI      string
    Password string
}

func (s *Service) StartType2Write(request Type2WriteStartRequestDTO) (TaskDTO, error)
func (s *Service) StartType2NDEFWrite(request Type2NDEFWriteStartRequestDTO) (TaskDTO, error)
```

Both use the existing operation slot, so reads and writes cannot overlap. They
call `WriteType2Pages` / `WriteType2NDEF`, emit progress with the written/
verified counts and backup path, and publish the refreshed Type 2 snapshot on
success. A failure keeps the error message, including the backup path, as the
task failure message.

NDEF Text and URI messages are built in `internal/type2`:

```go
func EncodeTextMessage(text, language string) ([]byte, error)
func EncodeURIMessage(uri string) ([]byte, error)
```

The encoder uses the short or long NDEF record layout as needed and applies a
known URI prefix when one matches. These functions only build the NDEF message;
TLV and capacity checks stay in `PlanNDEFWrites`.

## Desktop state

The shell gains:

- selected page index and page hex input;
- NDEF mode (`text`/`uri`) and message input;
- one pending write intent and an alert-dialog open flag.

The Memory tab:

- makes user pages selectable;
- shows the selected page number and editable hex input;
- rejects non-hex or non-four-byte input before opening the dialog;
- disables editing for system, lock, configuration, password, and PACK pages.

The NDEF tab:

- offers Text and URI modes;
- validates non-empty input;
- keeps the existing parsed-message display below the editor.

## Confirmation and failure UX

Every write opens `ui.AlertDialog` with:

- a title naming the operation;
- the target page and new hex, or the NDEF mode and byte length;
- a statement that a full-image backup is created first;
- a statement that the write is verified by read-back;
- Cancel and explicit Confirm write buttons.

The dialog is the only path to `StartType2Write` / `StartType2NDEFWrite`.
Canceling or pressing Escape clears the pending intent without starting work.

Task failures set the shell error banner from the task event message. The
message includes the backup path when a backup was written. A successful task
refreshes the Type 2 snapshot through the existing `Type2EventName` event.

## Tests

- NDEF Text/URI encoder round-trip, long payload, and URI prefix tests.
- Application task tests or CLI-independent service tests for the async
  wrappers, including a failed write with a backup path.
- Desktop tests:
  - page write opens the confirmation and submits the expected request;
  - NDEF write opens the confirmation and submits the expected request;
  - cancel starts nothing;
  - invalid hex and empty NDEF are rejected;
  - system pages cannot be edited;
  - a failed task sets the error banner.
- Existing hardware validation covers the write path; the GUI adds no new
  transport or card command.

## Non-goals

- No protected/configuration/PWD writes in the GUI.
- No Classic GUI write workflow.
- No arbitrary NDEF record editor.
- No direct file import/export in the NDEF editor; that remains CLI.
