# The undocumented Windows interfaces

Lanes depends on two Windows interfaces that Microsoft has never documented.
This page is for whoever has to fix one of them after a Windows update breaks
it. Everything else Lanes does uses documented Core Audio APIs.

| Interface | What Lanes uses it for | Code |
| --- | --- | --- |
| `IAudioPolicyConfigFactory` | Sending one application's audio to a chosen output device | `winaudio/src/policy.rs` |
| `IPolicyConfig` | Making a device Windows' default output (Master's device button) | `winaudio/src/default_device.rs` |

Both are what Windows' own UI uses: the first is behind **Settings > System >
Sound > Volume mixer**'s per-app output picker, the second behind "Set Default"
in the classic Sound control panel. Every tool that does either - EarTrumpet,
SoundSwitch, headset vendors' software - uses the same interfaces.

Their identifiers and method order were learned from
[EarTrumpet](https://github.com/File-New-Project/EarTrumpet)'s C# declarations.
Lanes' Rust bindings are an independent implementation; see
[THIRD-PARTY-NOTICES.md](../THIRD-PARTY-NOTICES.md).

---

## Per-application routing: `IAudioPolicyConfigFactory`

### How it is reached

It is a WinRT activation factory, not a classic COM class:

```text
RoGetActivationFactory("Windows.Media.Internal.AudioPolicyConfig", IID, &factory)
```

`RoGetActivationFactory` is imported from **`runtimeobject.lib`**, not
`combase.lib` (which the SDK does not ship, despite the function living in
`combase.dll`).

### The two IIDs

| IID | Windows |
| --- | --- |
| `ab3d4648-e242-459f-b02f-541c70306324` | Windows 11 21H2 and later (tested on build 26200) |
| `2a59116d-6c4f-45e0-a74f-707e3fef9258` | Windows 10, and Windows 11 before 21H2 |

`AudioPolicyConfig::connect` tries the newer one first, then the older, and
reports which was accepted. Asking is more reliable than guessing from the
Windows version.

### The vtable

The interface derives from `IInspectable`, and there is no metadata for it, so
the vtable is declared by hand (`AudioPolicyConfigVtbl`). **Every slot matters**:
the three methods Lanes needs are at the end, and a miscounted table calls the
wrong function pointer - which does not fail cleanly.

| Slot | Method |
| --- | --- |
| 0-2 | `IUnknown`: QueryInterface, AddRef, Release |
| 3-5 | `IInspectable`: GetIids, GetRuntimeClassName, GetTrustLevel |
| 6-24 | 19 methods Lanes never calls (volume groups, ringer state, chat application) |
| 25 | `SetPersistedDefaultAudioEndpoint(pid, flow, role, HSTRING deviceId)` |
| 26 | `GetPersistedDefaultAudioEndpoint(pid, flow, role, out HSTRING)` |
| 27 | `ClearAllPersistedApplicationDefaultEndpoints()` - **never called** |

### Three things that fail silently

1. **The device ID must be wrapped.** It is not the ID `IMMDevice::GetId`
   returns, but a device interface path:

   ```text
   \\?\SWD#MMDEVAPI#{0.0.0.00000000}.{guid}#{e6327cad-dcec-4949-ae8a-991e976a79d2}
   ```

   (the last GUID is `KSCATEGORY_AUDIO` render; capture uses
   `{2eef81be-33fa-4800-9670-1cd474972c3f}`). Pass the plain ID and the call
   returns success and does nothing.
2. **Both roles must be set**, `eConsole` and `eMultimedia`, or the app is left
   half-routed.
3. **An `HSTRING` is passed by handle, not by address.** `&hstring as *const _`
   is a pointer to the handle; the ABI wants the handle. Getting it wrong makes
   activation fail in a way indistinguishable from "unsupported".

A null `HSTRING` for the device clears that application's assignment, returning
it to the default device. That is how Restore undoes routing, one application at
a time.

### Never call `ClearAllPersistedApplicationDefaultEndpoints`

It clears every per-application assignment on the machine, including ones the
user made in Windows Settings, with no undo. It is declared only so the table
has the right shape; nothing wraps it.

### Assignments are persisted by Windows

A route set here is stored by Windows against the **application**, not the
process, and survives the application restarting, a reboot, and Lanes being
uninstalled. That is why Lanes has Restore.

---

## The default output device: `IPolicyConfig`

A classic COM interface, created with `CoCreateInstance`:

| | |
| --- | --- |
| CLSID (`CPolicyConfigClient`) | `870AF99C-171D-4F9E-AF0D-E63DF40C2BC9` |
| IID (`IPolicyConfig`) | `F8679F50-850A-41CF-9C72-430F290290C8` |

This IID has been stable from Windows 10 RS1 (and Windows 7/8) to today; two
early Windows 10 releases used others and are not supported.

Slot order after `IUnknown`: eight methods Lanes never calls, then
`GetPropertyValue`, `SetPropertyValue`, **`SetDefaultEndpoint(deviceId, role)`**,
`SetEndpointVisibility`. The device ID here is the plain `IMMDevice::GetId`
string - no wrapping.

Lanes sets the `eConsole` and `eMultimedia` roles, as "Set Default" does, and
leaves the communications role alone (it is a separate Windows setting some
people deliberately point at a headset). `set_default_output` then **reads the
default back** and reports an error if Windows did not take it.

---

## What "broken" looks like, and what to do

| Symptom | Likely cause |
| --- | --- |
| The mixer shows "Per-app device routing is unavailable" | `connect` failed. Its error lists the HRESULT for each IID tried |
| `REGDB_E_CLASSNOTREG` (0x80040154) | The runtime class or COM class is gone from this Windows |
| `E_NOINTERFACE` (0x80004002) | The class exists but the IID changed - look for a new one |
| `CO_E_NOTINITIALIZED` (0x800401F0) | Lanes' fault: COM not initialised on this thread |
| Calls succeed, audio does not move | Wrong slot order, or the device ID was not wrapped |
| Master's device button reports "Windows accepted the new default device but still reports ..." | `SetDefaultEndpoint` landed in the wrong slot, or Windows refused silently |

When routing is unavailable Lanes keeps working as a per-application volume
mixer and says so; it never crashes over this.

### Repairing it

1. Find the new IID or slot layout. EarTrumpet usually tracks Windows changes to
   these interfaces quickly; its `Interop` folder is the best place to look.
2. Change the constant or the table in `policy.rs` / `default_device.rs`.
3. **Verify by ear, not by return code.** Route a playing app to a different
   device and listen; read the route back with `GetPersistedDefaultAudioEndpoint`
   (Lanes' Diagnostics shows what each app is on). Every silent failure above
   returns success.

The `windows` crate is pinned to an exact version in `Cargo.toml` for the same
reason: a changed binding is a hidden variable when diagnosing this.

---

## COM apartment rules

Lanes joins a **single-threaded apartment** on every thread that touches audio,
and every audio call stays on the thread that joined it. The core runs all of
them on one thread (see [architecture.md](architecture.md)).

**Every interface must be released before `CoUninitialize`.** Releasing one
afterwards crashes the process at exit - after it has printed entirely correct
output, which makes it look like a Core Audio fault rather than an ordering one.
`winaudio::com::initialize` returns a guard; create it first so it drops last.
