using System.IO;
using System.Net.WebSockets;
using System.Text;
using System.Text.Json;
using System.Windows.Threading;

namespace Lanes.Ui.Api;

/// <summary>
/// The window's only connection to the rest of the application.
/// </summary>
/// <remarks>
/// <para>
/// The core runs in a separate process and owns every audio decision. This
/// class is the entire surface between them, which is what makes the Stream
/// Deck plugin nearly free: anything this window can do, a plugin can do, by
/// sending the same JSON.
/// </para>
/// <para>
/// Events are raised on the UI thread. The socket is read on a background task
/// and each message is marshalled across with the dispatcher, so nothing that
/// subscribes has to think about threading.
/// </para>
/// </remarks>
public sealed class CoreClient : IAsyncDisposable
{
    private static readonly JsonSerializerOptions Json = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
        PropertyNameCaseInsensitive = true,
        DefaultIgnoreCondition = System.Text.Json.Serialization.JsonIgnoreCondition.WhenWritingNull,
    };

    private readonly Dispatcher _dispatcher;
    private readonly CancellationTokenSource _closing = new();

    /// <summary>Replaced on every reconnect: a closed socket cannot be reused.</summary>
    private ClientWebSocket _socket = new();

    private string _token = "";
    private ulong _nextId = 1;

    /// <summary>Full state, on connect and after every command.</summary>
    public event Action<CoreState>? StateReceived;

    /// <summary>Only the sections that changed; the rest are null.</summary>
    public event Action<Incoming>? DeltaReceived;

    public event Action<IReadOnlyList<MeterLevel>>? MetersReceived;

    /// <summary>A command was refused. The argument is the core's own explanation.</summary>
    /// <remarks>
    /// Most of the window has no use for this - a fader that is refused simply
    /// snaps back when the next state arrives. It exists for settings, where the
    /// reason ("Shift+M needs Ctrl, Alt or Win with it") is the whole answer.
    /// </remarks>
    public event Action<string>? CommandRefused;

    /// <summary>The reply to <see cref="GetDiagnostics"/>.</summary>
    public event Action<Diagnostics>? DiagnosticsReceived;

    /// <summary>The reply to <see cref="ExportConfig"/>: the config as a file holds it.</summary>
    public event Action<JsonElement>? ConfigExported;

    /// <summary>The connection ended. The argument is worth showing the user.</summary>
    public event Action<string>? Disconnected;

    /// <summary>Connected, or reconnected after a drop.</summary>
    public event Action? Connected;

    /// <summary>
    /// The core is shutting down on purpose. Close; do not reconnect.
    /// </summary>
    /// <remarks>
    /// A dropped socket alone cannot tell a deliberate shutdown from a crash or
    /// a network blip, and the right response differs: one should be retried,
    /// the other should not. Without this the window sits saying
    /// "Reconnecting…" forever after the user quits from the tray - and, more
    /// practically, keeps its own executable locked, which is what stops an
    /// uninstaller deleting it.
    /// </remarks>
    public event Action? ShuttingDown;

    public CoreClient(Dispatcher dispatcher)
    {
        _dispatcher = dispatcher;
        lock (Live)
        {
            Live.Add(this);
        }
    }

    /// <summary>
    /// Where the core leaves its port number and shared token.
    /// </summary>
    /// <remarks>
    /// The port is written to a file precisely so clients do not have to be
    /// configured, and so a port collision is invisible to them. The token is
    /// in the config, which is outside the repository and always will be.
    /// <para>
    /// <b>Portable mode</b> is honoured the same way the core honours it: a
    /// <c>portable.txt</c> beside the executable puts everything in a
    /// <c>Lanes</c> folder beside it instead (<c>core/src/paths.rs</c>). Were
    /// the window to ignore the marker and look in <c>%LOCALAPPDATA%</c>
    /// regardless, a portable copy's window would find either nothing or
    /// somebody else's core.
    /// </para>
    /// </remarks>
    private static string StateFolder =>
        File.Exists(Path.Combine(AppContext.BaseDirectory, "portable.txt"))
            ? Path.Combine(AppContext.BaseDirectory, "Lanes")
            : Path.Combine(
                Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
                "Lanes");

    /// <summary>
    /// The theme the user last chose, read straight from the config file.
    /// </summary>
    /// <remarks>
    /// Only for drawing the first frame in the right colours, before there is
    /// a connection to ask - the same file <see cref="Discover"/> already reads
    /// the token from. The core's own answer replaces it on the first state
    /// push. Anything unreadable means "follow Windows".
    /// </remarks>
    public static string? SavedTheme()
    {
        try
        {
            var configFile = Path.Combine(StateFolder, "config.json");
            using var document = JsonDocument.Parse(File.ReadAllText(configFile));
            return document.RootElement.TryGetProperty("settings", out var settings)
                && settings.TryGetProperty("theme", out var theme)
                ? theme.GetString()
                : null;
        }
        catch
        {
            return null;
        }
    }

    private static (int Port, string Token) Discover()
    {
        var portFile = Path.Combine(StateFolder, "port");
        var configFile = Path.Combine(StateFolder, "config.json");

        if (!File.Exists(portFile))
        {
            throw new InvalidOperationException(
                "Lanes is not running. Start it from the Start menu or the tray.");
        }

        var port = int.Parse(File.ReadAllText(portFile).Trim());

        // ReadAllText strips a byte order mark. Worth relying on deliberately:
        // a BOM in this file has broken the core's own parser before.
        var token = "";
        if (File.Exists(configFile))
        {
            using var document = JsonDocument.Parse(File.ReadAllText(configFile));
            if (document.RootElement.TryGetProperty("api_token", out var value))
            {
                token = value.GetString() ?? "";
            }
        }

        return (port, token);
    }

    /// <summary>
    /// Connect, and keep reconnecting for as long as the window is open.
    /// </summary>
    /// <remarks>
    /// The core outlives this window by design — closing the window leaves it
    /// running in the tray — but it can also be restarted underneath one, which
    /// happens constantly during development and to a user after an update.
    /// Without this the window would sit showing a stale mixer and a disconnect
    /// message, with every control silently doing nothing.
    ///
    /// The first attempt is awaited so a genuine problem is reported
    /// immediately rather than retried in silence.
    /// </remarks>
    public async Task ConnectAsync()
    {
        try
        {
            await OpenAsync();
        }
        catch (Exception) when (CoreProcess.Path is not null)
        {
            // No core. Start one rather than telling the user to.
            //
            // This is what makes a window pinned to the taskbar work: Windows
            // pins the executable of the window you are looking at, which is
            // this one, and the pin would otherwise open a mixer with nothing
            // behind it. "Lanes is not running. Start it from the Start menu"
            // would be a true sentence and a useless one - the person did.
            await StartCoreAndWaitAsync();
        }

        _ = Task.Run(SuperviseAsync);
    }

    /// <summary>
    /// Start the core, then keep trying to connect until it is listening.
    /// </summary>
    /// <remarks>
    /// <para>
    /// The core has real work to do before it accepts a connection - a COM
    /// apartment, the config, the first-run snapshot on a new machine - so
    /// there is no moment at which "it has started" and "it is listening" are
    /// the same thing. Polling is the honest way to wait for that, and the
    /// budget is generous because the alternative to waiting a second too long
    /// is reporting a failure that has not happened.
    /// </para>
    /// <para>
    /// The retry also covers a stale <c>port</c> file, left behind by a core
    /// that died rather than quit. That case looks like "running" to
    /// <c>Discover</c> and fails at the socket, so both have to be retried
    /// rather than only the missing-file one.
    /// </para>
    /// </remarks>
    private async Task StartCoreAndWaitAsync()
    {
        if (!CoreProcess.Start())
        {
            throw new InvalidOperationException(
                "Lanes could not start its core. Reinstall, or run Lanes.exe directly.");
        }

        var deadline = DateTime.UtcNow + TimeSpan.FromSeconds(20);
        Exception? last = null;

        while (DateTime.UtcNow < deadline)
        {
            await Task.Delay(250, _closing.Token);

            try
            {
                await OpenAsync();
                return;
            }
            catch (Exception e)
            {
                last = e;
            }
        }

        throw new InvalidOperationException(
            "Lanes started its core but it did not answer. See the log folder.",
            last);
    }

    private async Task OpenAsync()
    {
        var (port, token) = Discover();
        _token = token;

        _socket = new ClientWebSocket();
        await _socket.ConnectAsync(new Uri($"ws://127.0.0.1:{port}/"), _closing.Token);
        // Discarded explicitly: inside an async method the compiler is right to
        // ask whether an un-awaited awaitable was a mistake. Here it is not —
        // the event is raised on the UI thread and nothing waits for it.
        _ = _dispatcher.BeginInvoke(() => Connected?.Invoke());
    }

    /// <summary>Read until the connection drops, then wait and try again.</summary>
    private async Task SuperviseAsync()
    {
        while (!_closing.IsCancellationRequested)
        {
            await ReadLoopAsync();

            if (_closing.IsCancellationRequested)
            {
                return;
            }

            _socket.Dispose();

            // Long enough not to spin against a core that is gone, short enough
            // that a restart is picked up before anyone reaches for the mouse.
            try
            {
                await Task.Delay(TimeSpan.FromSeconds(2), _closing.Token);
            }
            catch (OperationCanceledException)
            {
                return;
            }

            try
            {
                await OpenAsync();
            }
            catch (Exception)
            {
                // Still not there. The loop waits and tries again; the message
                // already on screen is still accurate.
            }
        }
    }

    /// <summary>Send a command. The caller does not wait: the reply arrives as state.</summary>
    /// <remarks>
    /// Every command returns the resulting state and the read loop applies it
    /// like any other push, so the caller never waits. A client that awaited
    /// each command would serialise a drag into a round trip per step. Sends
    /// still leave in order, and each is tracked until its reply arrives, so
    /// the process can wait for them before it exits (see <see cref="FlushAll"/>).
    /// </remarks>
    public void Send(Dictionary<string, object?> command)
    {
        command["token"] = _token;
        var id = _nextId++;
        command["id"] = id;
        lock (_awaiting)
        {
            _awaiting.Add(id);
        }

        var json = JsonSerializer.Serialize(command, Json);
        var bytes = Encoding.UTF8.GetBytes(json);

        // Queued rather than awaited, so the caller - usually a mouse move -
        // never blocks on the socket. And queued IN ORDER, one at a time.
        //
        // Two reasons. A WebSocket allows one send in flight at a time, so two
        // close together - a fast fader drag - would collide, and the loser
        // would fail silently. And a window that sends a command and closes -
        // the new-app notice's buttons, the mixer quitting when Close to tray
        // is off - must not exit before the send has left; `FlushAll` waits on
        // this chain.
        lock (_sendLock)
        {
            _sending = _sending.ContinueWith(
                async _ =>
                {
                    try
                    {
                        await _socket.SendAsync(bytes, WebSocketMessageType.Text, true, _closing.Token);
                    }
                    catch (Exception)
                    {
                        // The read loop reports the disconnect; a failed send
                        // does not need to report it a second time.
                    }
                },
                TaskScheduler.Default).Unwrap();
        }
    }

    private readonly object _sendLock = new();

    /// <summary>Every send queued so far, in order. See <see cref="Send"/>.</summary>
    private Task _sending = Task.CompletedTask;

    /// <summary>
    /// Commands sent whose reply has not come back yet, by id.
    /// </summary>
    /// <remarks>
    /// Waiting for the send alone is not enough to exit safely. Lanes keeps
    /// pushing updates to every client, so a window always has unread data
    /// in its socket, and a process that exits with unread data makes Windows
    /// <i>reset</i> the connection rather than close it - which throws away
    /// whatever the core had not yet read. The core reads each connection every
    /// few milliseconds to a tenth of a second, so a command sent just before
    /// the exit would be lost more often than not. The reply is the only proof
    /// the core has it.
    /// </remarks>
    private readonly HashSet<ulong> _awaiting = [];

    /// <summary>Every client in this process, so they can all be flushed on exit.</summary>
    private static readonly List<CoreClient> Live = [];

    /// <summary>
    /// Wait, up to <paramref name="max"/>, for every command every client has
    /// queued to leave. Called as the process exits: WPF ends the process as
    /// soon as the last window closes, and would otherwise take unsent
    /// commands with it.
    /// </summary>
    public static void FlushAll(TimeSpan max)
    {
        CoreClient[] clients;
        lock (Live)
        {
            clients = [.. Live];
        }

        // Until every command sent has been answered - see `_awaiting` - or the
        // time is up. The read loop runs on the thread pool, so waiting here,
        // on the UI thread, does not stop the replies arriving.
        var deadline = DateTime.UtcNow + max;
        while (DateTime.UtcNow < deadline)
        {
            var outstanding = clients.Any(c =>
            {
                lock (c._awaiting)
                {
                    return c._awaiting.Count > 0 && c.IsConnected;
                }
            });
            if (!outstanding)
            {
                return;
            }
            Thread.Sleep(10);
        }
    }

    public void SetChannelVolume(string channel, double value) =>
        Send(new() { ["command"] = "set_channel_volume", ["channel"] = channel, ["value"] = value });

    public void ToggleChannelMute(string channel) =>
        Send(new() { ["command"] = "toggle_channel_mute", ["channel"] = channel });

    public void SetChannelDevice(string channel, string? deviceId) =>
        Send(new() { ["command"] = "set_channel_device", ["channel"] = channel, ["device_id"] = deviceId });

    /// <summary>Replace the global device fallback order.</summary>
    public void SetDevicePriority(IReadOnlyList<(string Id, bool Enabled)> entries) =>
        Send(new()
        {
            ["command"] = "set_device_priority",
            ["entries"] = entries.Select(e => new Dictionary<string, object?>
            {
                ["id"] = e.Id,
                ["enabled"] = e.Enabled,
            }).ToList(),
        });

    public void ActivateProfile(string name) =>
        Send(new() { ["command"] = "activate_profile", ["name"] = name });

    public void SaveProfile(string name) =>
        Send(new() { ["command"] = "save_profile", ["name"] = name });

    public void DeleteProfile(string name) =>
        Send(new() { ["command"] = "delete_profile", ["name"] = name });

    public void SetChatMix(double value) =>
        Send(new() { ["command"] = "set_chat_mix", ["value"] = value });

    public void AssignApp(string executable, string channel) =>
        Send(new() { ["command"] = "assign_app", ["executable"] = executable, ["channel"] = channel });

    public void UnassignApp(string executable) =>
        Send(new() { ["command"] = "unassign_app", ["executable"] = executable });

    /// <summary>
    /// Show or hide a channel's application list.
    /// </summary>
    /// <remarks>
    /// Display state, sent to the core rather than kept here, because this
    /// window is destroyed every time it closes and the collapse state has to
    /// survive that.
    /// </remarks>
    public void SetChannelCollapsed(string channel, bool collapsed) =>
        Send(new() { ["command"] = "set_channel_collapsed", ["channel"] = channel, ["collapsed"] = collapsed });

    /// <summary>Remember where the window is. Sent as it closes.</summary>
    public void SetWindowPlacement(int x, int y, int width, int height, bool maximised) =>
        Send(new()
        {
            ["command"] = "set_window_placement",
            ["x"] = x,
            ["y"] = y,
            ["width"] = width,
            ["height"] = height,
            ["maximised"] = maximised,
        });

    /// <summary>Replace every global hotkey. The core validates and normalises.</summary>
    public void SetHotkeys(IEnumerable<(string Keys, HotkeyAction Action)> bindings) =>
        Send(new()
        {
            ["command"] = "set_hotkeys",
            ["bindings"] = bindings.Select(b =>
            {
                var action = new Dictionary<string, object?> { ["kind"] = b.Action.Kind };
                if (b.Action.Channel is { } channel)
                {
                    action["channel"] = channel;
                }
                if (b.Action.Name is { } name)
                {
                    action["name"] = name;
                }
                return new Dictionary<string, object?> { ["keys"] = b.Keys, ["action"] = action };
            }).ToList(),
        });

    public void AddChannel(string name) =>
        Send(new() { ["command"] = "add_channel", ["name"] = name });

    public void RemoveChannel(string channel) =>
        Send(new() { ["command"] = "remove_channel", ["channel"] = channel });

    public void RenameChannel(string channel, string name) =>
        Send(new() { ["command"] = "rename_channel", ["channel"] = channel, ["name"] = name });

    public void IgnoreApp(string executable) =>
        Send(new() { ["command"] = "ignore_app", ["executable"] = executable });

    public void UnignoreApp(string executable) =>
        Send(new() { ["command"] = "unignore_app", ["executable"] = executable });

    /// <summary>An app's level within its channel; null follows the channel.</summary>
    public void SetAppTrim(string executable, double? trim) =>
        Send(new() { ["command"] = "set_app_trim", ["executable"] = executable, ["trim"] = trim });

    public void GetDiagnostics() => Send(new() { ["command"] = "get_diagnostics" });

    public void ExportConfig() => Send(new() { ["command"] = "export_config" });

    public void ImportConfig(JsonElement config) =>
        Send(new() { ["command"] = "import_config", ["config"] = config });

    public void SetNotifyNewApps(bool enabled) =>
        Send(new() { ["command"] = "set_notify_new_apps", ["enabled"] = enabled });

    /// <summary>
    /// Put Windows audio back as it was before Lanes - every app at 100%,
    /// unmuted, on the default device - and pause Lanes.
    /// </summary>
    public void RestoreWindows() => Send(new() { ["command"] = "restore_windows_settings" });

    /// <summary>Stop being paused: Lanes manages applications again.</summary>
    public void Resume() => Send(new() { ["command"] = "resume" });

    /// <summary>Quit Lanes: the core, the tray, every window.</summary>
    public void Quit() => Send(new() { ["command"] = "quit" });

    /// <summary>Balance these two channels in the mix. It returns to centre.</summary>
    public void SetChatMixChannels(string game, string chat) =>
        Send(new() { ["command"] = "set_chat_mix_channels", ["game"] = game, ["chat"] = chat });

    /// <param name="theme">"system", "dark" or "light".</param>
    public void SetTheme(string theme) =>
        Send(new() { ["command"] = "set_theme", ["theme"] = theme });

    /// <summary>Ask the core to open the mixer, or bring it forward if it is open.</summary>
    public void ShowWindow() =>
        Send(new() { ["command"] = "show_window" });

    public void SubscribeMeters(bool enabled) =>
        Send(new() { ["command"] = "subscribe_meters", ["enabled"] = enabled });

    private async Task ReadLoopAsync()
    {
        var buffer = new byte[64 * 1024];
        var message = new MemoryStream();

        try
        {
            while (_socket.State == WebSocketState.Open && !_closing.IsCancellationRequested)
            {
                message.SetLength(0);
                WebSocketReceiveResult result;

                // A message can arrive in several frames. Reassembling is not
                // optional: state pushes are comfortably larger than one frame.
                do
                {
                    result = await _socket.ReceiveAsync(buffer, _closing.Token);
                    if (result.MessageType == WebSocketMessageType.Close)
                    {
                        Report("the core closed the connection");
                        return;
                    }
                    message.Write(buffer, 0, result.Count);
                }
                while (!result.EndOfMessage);

                Handle(Encoding.UTF8.GetString(message.GetBuffer(), 0, (int)message.Length));
            }
        }
        catch (OperationCanceledException)
        {
            // Closing normally.
        }
        catch (Exception e)
        {
            Report(e.Message);
        }
    }

    /// <summary>True while the socket can actually carry a command.</summary>
    public bool IsConnected => _socket.State == WebSocketState.Open;

    private void Handle(string json)
    {
        Incoming? incoming;
        try
        {
            incoming = JsonSerializer.Deserialize<Incoming>(json, Json);
        }
        catch (JsonException)
        {
            // A message we cannot read is not worth tearing the connection down
            // for; the next push will be a full state anyway.
            return;
        }

        if (incoming is null)
        {
            return;
        }

        // A reply: the core has this command. Recorded here, on the read
        // thread, so an exit waiting on the UI thread still sees it.
        if (incoming.Id is { } answered)
        {
            lock (_awaiting)
            {
                _awaiting.Remove(answered);
            }
        }

        _dispatcher.BeginInvoke(() =>
        {
            if (incoming.Levels is { } levels)
            {
                MetersReceived?.Invoke(levels);
                return;
            }

            // Replies that carry something besides the state. Raised before the
            // state is applied, and the state still is.
            if (incoming.Diagnostics is { } diagnostics)
            {
                DiagnosticsReceived?.Invoke(diagnostics);
            }
            if (incoming.Config is { } exported)
            {
                ConfigExported?.Invoke(exported);
            }

            // A command reply carries the state after the command ran, so it is
            // applied exactly like a push. That is what keeps this window from
            // ever having to guess whether something worked.
            if (incoming.State is { } state)
            {
                StateReceived?.Invoke(state);
                return;
            }

            if (incoming.Ok == false && incoming.Error?.Message is { } refused)
            {
                CommandRefused?.Invoke(refused);
                return;
            }

            if (incoming.Event == "shutdown")
            {
                // Stop the reconnect loop first. Otherwise the socket closing
                // behind this event looks like a fault worth retrying.
                _closing.Cancel();
                ShuttingDown?.Invoke();
                return;
            }

            if (incoming.Event == "delta")
            {
                DeltaReceived?.Invoke(incoming);
            }
        });
    }

    private void Report(string why) => _dispatcher.BeginInvoke(() => Disconnected?.Invoke(why));

    public async ValueTask DisposeAsync()
    {
        // What was sent is answered first - cancelling would abandon it (see
        // _awaiting).
        var until = DateTime.UtcNow.AddMilliseconds(500);
        while (DateTime.UtcNow < until && IsConnected)
        {
            lock (_awaiting)
            {
                if (_awaiting.Count == 0)
                {
                    break;
                }
            }
            await Task.Delay(10);
        }
        lock (Live)
        {
            Live.Remove(this);
        }

        await _closing.CancelAsync();
        try
        {
            if (_socket.State == WebSocketState.Open)
            {
                await _socket.CloseAsync(WebSocketCloseStatus.NormalClosure, null, CancellationToken.None);
            }
        }
        catch (Exception)
        {
            // Shutting down; a socket that will not close cleanly is not worth
            // delaying the exit for.
        }
        _socket.Dispose();
        _closing.Dispose();
    }
}
