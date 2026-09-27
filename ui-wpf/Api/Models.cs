using System.Text.Json.Serialization;

namespace Lanes.Ui.Api;

// The wire format is documented in docs/api.md and is owned by the core. These
// are a transcription of it, not a second definition: if the two disagree, the
// core is right.
//
// Every name here is snake_case on the wire. That is handled once, by the
// serializer options in CoreClient, rather than with an attribute per property.

/// <summary>A device, as referenced by a channel.</summary>
public sealed record DeviceRef(string Id, string Name);

/// <summary>An audio endpoint.</summary>
/// <param name="Direction">"output" or "input". The Mic strip must filter on
/// this — offering speakers as a microphone was a real bug in the last UI.</param>
public sealed record DeviceState(
    string Id,
    string Name,
    string Direction,
    bool Present,
    bool IsDefault)
{
    public bool IsOutput => Direction == "output";
}

/// <summary>One mixer channel.</summary>
public sealed record ChannelState(
    string Id,
    string Name,
    /// <summary>What the user set. A command that sets a volume sets this.</summary>
    float Volume,
    /// <summary>
    /// What the channel is actually running at, once Master's ceiling applies.
    /// </summary>
    /// <remarks>
    /// Master limits every playback channel rather than scaling it, so a
    /// channel set to 100% with Master at 75% runs at 75% and still remembers
    /// that it was set to 100%. <b>This is what a fader should show.</b>
    /// </remarks>
    float EffectiveVolume,
    bool Muted,
    bool IsInput,
    DeviceRef? TargetDevice,
    DeviceRef? EffectiveDevice,
    /// <summary>
    /// The preferred device is absent and this channel is on a fallback.
    /// Explicit rather than inferred by comparing the two device fields,
    /// because a channel silently playing out of the wrong speakers is the
    /// worst failure this application can have.
    /// </summary>
    bool OnFallback,
    /// <summary>
    /// Whether this channel's application list is shown collapsed.
    /// </summary>
    /// <remarks>
    /// Display state, and it comes from the core because it is remembered
    /// between sessions - and this window is
    /// destroyed every time it closes, so it cannot remember anything itself.
    /// </remarks>
    bool Collapsed,
    /// <summary>Only a channel that was added can be removed; the six that ship cannot.</summary>
    bool Removable = false);

/// <summary>An application making sound.</summary>
/// <param name="Channel">null means it is in the "To be routed" pool, playing
/// at its own volume outside the user's control.</param>
/// <param name="SessionCount">Browsers and Electron apps own several sessions;
/// the core groups them so every client does not have to.</param>
/// <param name="Trim">Its level within its channel, when set below the channel;
/// null means it follows the channel exactly.</param>
public sealed record SessionState(
    string Executable,
    string DisplayName,
    string? Channel,
    bool Playing,
    bool Controllable,
    int SessionCount,
    float? Trim = null,
    string? Path = null,
    bool RoutingRefused = false);

/// <summary>The Game/Chat balance.</summary>
/// <param name="Value">-1 to 1, where 0 is centre and attenuates nothing.</param>
/// <param name="Game">Channel id favoured at +1. Not assumed to be "game" —
/// channels are renameable.</param>
/// <param name="Chat">Channel id favoured at -1.</param>
public sealed record ChatMixState(float Value, string Game, string Chat)
{
    public static ChatMixState Centred { get; } = new(0f, "game", "chat");
}

/// <summary>Saved profiles.</summary>
/// <param name="Names">Every profile, in the order they were created.</param>
/// <param name="Active">The profile the mixer currently <b>matches</b>, or null.
/// Moving any fader a profile holds makes this null; the core decides, not the
/// window.</param>
public sealed record ProfileState(IReadOnlyList<string> Names, string? Active);

/// <summary>Where the window was last left.</summary>
/// <remarks>
/// Physical pixels on the virtual screen, which is what Win32 window placement
/// deals in. Not logical units: a logical size is meaningless without saying
/// which monitor's scaling it was logical in.
/// </remarks>
public sealed record WindowPlacement(int X, int Y, int Width, int Height, bool Maximised);

/// <summary>Everything the core knows, as pushed on connect.</summary>
public sealed record CoreState(
    /// <summary>Null until the window has been closed at least once.</summary>
    WindowPlacement? Window,
    IReadOnlyList<ChannelState> Channels,
    IReadOnlyList<SessionState> Sessions,
    IReadOnlyList<DeviceState> Devices,
    ProfileState? Profiles,
    ChatMixState? ChatMix,
    /// <summary>
    /// False means per-app routing is unavailable on this Windows build. The
    /// app still works as a volume mixer and must say so plainly.
    /// </summary>
    bool RoutingAvailable,
    /// <summary>
    /// The global device fallback order. Optional so an older core that does
    /// not send it still deserialises: binding is by name, and a missing name
    /// takes the default rather than failing.
    /// </summary>
    IReadOnlyList<PriorityState>? DevicePriority = null,
    /// <summary>Global hotkeys and whether each is live. Optional for the same reason.</summary>
    IReadOnlyList<HotkeyState>? Hotkeys = null,
    /// <summary>Executables never managed or shown.</summary>
    IReadOnlyList<string>? Ignored = null,
    SettingsState? Settings = null);

/// <summary>Preferences a client can change.</summary>
/// <param name="Theme">"system", "dark" or "light". See <see cref="Ui.Themes"/>.</param>
/// <param name="CloseToTray">When false, closing the mixer quits Lanes.</param>
/// <param name="Paused">Lanes restored Windows audio and is leaving it alone
/// until resumed.</param>
public sealed record SettingsState(bool NotifyNewApps, string Theme = "system", bool CloseToTray = true, bool Paused = false);

/// <summary>The reply to <c>get_diagnostics</c>.</summary>
public sealed record Diagnostics(
    string StateFolder,
    string LogFolder,
    bool Portable,
    bool RoutingAvailable,
    IReadOnlyList<DiagnosticSession> Sessions,
    IReadOnlyList<System.Text.Json.JsonElement> Recent);

/// <summary>One audio session as the core sees it, and why.</summary>
public sealed record DiagnosticSession(
    string Executable,
    string Path,
    uint ProcessId,
    string State,
    float Volume,
    bool Muted,
    string? Channel,
    string? Rule,
    bool Ignored);

/// <summary>What a hotkey does.</summary>
/// <param name="Kind">
/// <c>toggle_mute</c>, <c>volume_up</c>, <c>volume_down</c>, <c>cycle_device</c>
/// (these four name a <paramref name="Channel"/>), <c>activate_profile</c>
/// (names a profile in <paramref name="Name"/>), <c>mute_all</c>,
/// <c>show_window</c> and <c>quick_mixer</c>.
/// </param>
public sealed record HotkeyAction(string Kind, string? Channel = null, string? Name = null);

/// <summary>One global hotkey.</summary>
/// <param name="Keys">Normalised by the core: <c>Ctrl+Alt+M</c>.</param>
/// <param name="Status">
/// <c>active</c>, <c>in_use</c> (another program holds the keys), <c>failed</c>
/// or <c>pending</c> (the tray has not reported yet).
/// </param>
public sealed record HotkeyState(string Keys, HotkeyAction Action, string Status, string? Problem);

/// <summary>One output device's place in the fallback order.</summary>
public sealed record PriorityState(string Id, string Name, bool Enabled, bool Present);

public sealed record MeterLevel(string Channel, float Level);

/// <summary>An event pushed by the core, or a reply to a command.</summary>
/// <remarks>
/// One shape covers both because they arrive on the same socket and are
/// distinguished by which fields are present: events carry "event", replies
/// carry "ok".
/// </remarks>
public sealed class Incoming
{
    [JsonPropertyName("event")] public string? Event { get; set; }
    public bool? Ok { get; set; }
    public ulong? Id { get; set; }
    public bool? Authenticated { get; set; }
    public CoreState? State { get; set; }
    public ApiError? Error { get; set; }

    // Delta: sections that did not change are omitted entirely, so these are
    // nullable and "absent" is meaningfully different from "empty".
    public IReadOnlyList<ChannelState>? Channels { get; set; }
    public IReadOnlyList<SessionState>? Sessions { get; set; }
    public IReadOnlyList<DeviceState>? Devices { get; set; }
    public IReadOnlyList<PriorityState>? DevicePriority { get; set; }
    public ProfileState? Profiles { get; set; }
    public ChatMixState? ChatMix { get; set; }
    public IReadOnlyList<HotkeyState>? Hotkeys { get; set; }
    public IReadOnlyList<string>? Ignored { get; set; }
    public SettingsState? Settings { get; set; }

    // Only in the replies to get_diagnostics and export_config.
    public Diagnostics? Diagnostics { get; set; }
    public System.Text.Json.JsonElement? Config { get; set; }

    public IReadOnlyList<MeterLevel>? Levels { get; set; }
}

public sealed class ApiError
{
    public string? Code { get; set; }
    public string? Message { get; set; }
}
