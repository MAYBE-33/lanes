using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;

using Lanes.Ui.Api;
using Lanes.Ui.Controls;

namespace Lanes.Ui;

/// <summary>
/// The mixer window.
/// </summary>
/// <remarks>
/// <para>
/// <b>The rule this window is built around:</b> controls are created when the
/// set of channels changes, and only then. Every state push after that updates
/// existing controls in place. Rebuilding the strip list on each push would
/// destroy the control the user was dragging roughly once per drag step — see
/// <see cref="ChannelStrip"/>.
/// </para>
/// </remarks>
public partial class MainWindow : Window, IAppCommands, IChannelCommands
{
    private readonly CoreClient _client;
    private readonly Dictionary<string, ChannelStrip> _strips = [];
    private readonly ChatMixSlider _chatMix = new();
    private readonly TextBlock _mixReadout = new();

    /// <summary>
    /// The two mixed channels' levels before the mix applies, and their names.
    /// </summary>
    /// <remarks>
    /// Kept so the readout can be recomputed while the control is being
    /// dragged, which is exactly when someone wants to see what it is doing —
    /// and is before the core has been told anything.
    /// </remarks>
    private (string Name, double Level) _mixLeft = ("", 1);
    private (string Name, double Level) _mixRight = ("", 1);

    /// <summary>
    /// Whether the channel drawn on the LEFT of the mix control is the one the
    /// core calls `game`.
    /// </summary>
    /// <remarks>
    /// This is the translation between two different ideas of direction, and
    /// getting it wrong inverts the whole control.
    ///
    /// The control is a position: left or right, toward one label or the other.
    /// The core's value is a preference: **positive favours `chat_mix.game`**
    /// by quietening `chat`. Those only line up if the game channel happens to
    /// be on the right, and it is not — the strips are ordered Game then Chat,
    /// so Game is the LEFT label. Passing the slider's position straight
    /// through would turn Game down when the user pulled toward Game, which is
    /// exactly backwards.
    ///
    /// The channel ids come from the core and the order comes from the strip
    /// layout, so neither can be assumed. This is worked out from both.
    /// </remarks>
    private bool _mixLeftIsGame = true;

    /// <summary>The core's value for a slider position, and back again.</summary>
    /// <remarks>
    /// Its own inverse, which is worth noting: there is one conversion here,
    /// not two that could disagree.
    /// </remarks>
    private double MixAcross(double value) => _mixLeftIsGame ? -value : value;
    private readonly DeviceButton _override = new();

    /// <summary>
    /// The profile picker. Its own card in the wide layout; inside the
    /// override card in the narrow ones. See <see cref="PlaceProfile"/>.
    /// </summary>
    private readonly ProfileButton _profiles = new();
    private readonly PoolPanel _pool = new();

    /// <summary>
    /// "Audio devices": the global device fallback order. Shares the pool's
    /// column in the wide layout, half each, and sits beside the pool's button
    /// in the narrow ones.
    /// </summary>
    private readonly DevicePanel _devicePanel = new();

    /// <summary>
    /// Connection state, shown in the header band's first column.
    /// </summary>
    /// <remarks>
    /// It had a row of its own across the top of the window, which cost every
    /// strip 34 logical pixels of height to display a string that is empty
    /// almost all the time. It now sits in the gap to the left of the mix
    /// control - space that already existed and was doing nothing.
    /// </remarks>
    private readonly TextBlock Status = new()
    {
        VerticalAlignment = VerticalAlignment.Bottom,
        TextWrapping = TextWrapping.Wrap,
        Margin = new Thickness(2, 0, 8, 16),
    };

    private IReadOnlyList<string> _channelOrder = [];

    /// <summary>The layout currently on screen. See <see cref="Layout"/>.</summary>
    private LayoutMode _mode = LayoutMode.Full;

    /// <summary>
    /// Whether the saved size and position have been applied yet.
    /// </summary>
    /// <remarks>
    /// Once only, and from the first state push rather than from
    /// <c>OnSourceInitialized</c>, because the placement lives in the core and
    /// arrives over the API - there is nothing to restore from until the
    /// connection is up. Applying it again on a later push would yank the
    /// window back while the user was moving it.
    /// </remarks>
    private bool _placementRestored;

    /// <summary>
    /// The channels as the core last reported them.
    /// </summary>
    /// <remarks>
    /// Held because a reflow rebuilds the header band, and the band needs the
    /// channels' names and order. A resize is not a state push, so there is
    /// nothing else to get them from at that moment.
    /// </remarks>
    private IReadOnlyList<ChannelState> _channels = [];

    /// <summary>Whether the header band is currently on two lines.</summary>
    /// <remarks>
    /// Held because <see cref="BandNeedsTwoLines"/> is sticky in the same way
    /// <see cref="Layout"/> is, and because this breakpoint sits *inside* the
    /// stacked layout - it can be crossed without the layout mode changing at
    /// all, so a reflow is not enough to notice it.
    /// </remarks>
    private bool _bandTwoLines;
    private IReadOnlyList<DeviceState> _devices = [];

    /// <summary>The sessions as last reported, so a confirmation can say which
    /// apps actually moved rather than which were merely dropped.</summary>
    private IReadOnlyList<SessionState> _sessions = [];

    /// <summary>
    /// "Close to tray", as the core last reported it. On by default; off, the
    /// X quits Lanes.
    /// </summary>
    /// <remarks>
    /// Acted on here rather than in the core: the core cannot tell a window
    /// closed with X from one that exited for another reason, so the window is
    /// the one that knows.
    /// </remarks>
    private bool _closeToTray = true;

    /// <summary>
    /// This close is Lanes' own doing - the core shutting down, or a rebuild
    /// for a theme change - and must not be taken as the user closing it.
    /// </summary>
    private bool _closingForLanes;

    /// <summary>
    /// The brief line that confirms a rule: "Discord.exe → Chat".
    /// </summary>
    /// <remarks>
    /// A drop creates a <i>persistent rule</i>, which is more than the chip
    /// moving suggests - and a chip moving into a collapsed strip is not
    /// visible at all. Expanding the strip to show it would break this window's
    /// standing rule that nothing expands itself; this line says the same thing
    /// without rearranging anything.
    ///
    /// Refusals use the same line, in amber, so a rule the core would not make
    /// is never confirmed by silence.
    /// </remarks>
    private readonly Border _toast = new()
    {
        HorizontalAlignment = HorizontalAlignment.Center,
        VerticalAlignment = VerticalAlignment.Bottom,
        Margin = new Thickness(0, 0, 0, 18),
        Padding = new Thickness(14, 8, 14, 8),
        CornerRadius = new CornerRadius(9),
        BorderThickness = new Thickness(1),
        IsHitTestVisible = false,
        Opacity = 0,
    };

    private readonly TextBlock _toastText = new();

    private readonly System.Windows.Threading.DispatcherTimer _toastTimer = new()
    {
        Interval = TimeSpan.FromSeconds(2.4),
    };
    private ChatMixState _chatMixState = ChatMixState.Centred;

    /// <summary>
    /// Where the window being replaced was, when this one is its replacement
    /// after a theme change. Applied before the window is first shown, so the
    /// new one appears exactly over the old rather than at its default.
    /// </summary>
    private readonly Placement.Bounds? _carried;

    public MainWindow() : this(null)
    {
    }

    private MainWindow(Placement.Bounds? carried)
    {
        _carried = carried;
        _placementRestored = carried is not null;

        InitializeComponent();
        Title = CoreProcess.Titled(Title);

        _client = new CoreClient(Dispatcher);
        _client.StateReceived += Apply;
        _client.DeltaReceived += ApplyDelta;
        _client.MetersReceived += ApplyMeters;
        _client.Disconnected += why =>
        {
            Status.Text = $"Reconnecting… ({why})";
            Status.Foreground = (Brush)Theme["Warn"];
        };

        // The core said it is going. Follow it: this window is a client and
        // there is nothing left to be a client of.
        _client.ShuttingDown += () =>
        {
            _closingForLanes = true;
            Close();
        };

        _client.Connected += () =>
        {
            Status.Text = "";
            Status.Foreground = (Brush)Theme["TextFaint"];
            // Re-asked on every connection: the core forgets its subscribers
            // when it restarts, and a window with dead meters looks broken.
            _client.SubscribeMeters(true);
        };

        _override.DeviceChosen += OverrideAll;

        // Every app chip's right-click menu - in the strips and in the pool -
        // asks this window, which asks the core.
        AppChip.Commands = this;
        ChannelStrip.Commands = this;
        ResumeButton.Click += (_, _) => _client.Resume();

        _toast.Background = (Brush)Theme["PopupGround"];
        _toast.BorderBrush = (Brush)Theme["LineStrong"];
        _toast.Child = _toastText;
        Grid.SetRowSpan(_toast, 2);
        Root.Children.Add(_toast);
        _toastTimer.Tick += (_, _) =>
        {
            _toastTimer.Stop();
            _toast.BeginAnimation(
                OpacityProperty,
                new System.Windows.Media.Animation.DoubleAnimation(0, TimeSpan.FromMilliseconds(260)));
        };
        _client.CommandRefused += why => Confirm(why, warn: true);

        // A plain click anywhere but a chip lets a Ctrl selection go, and so
        // does Esc. A chip handles its own clicks.
        PreviewMouseLeftButtonDown += (_, e) =>
        {
            if (!Keyboard.Modifiers.HasFlag(ModifierKeys.Control)
                && !InsideChip(e.OriginalSource as DependencyObject))
            {
                AppChip.ClearSelection();
            }
        };
        PreviewKeyDown += (_, e) =>
        {
            if (e.Key == Key.Escape)
            {
                AppChip.ClearSelection();
            }
        };

        _profiles.ActivateRequested += name => _client.ActivateProfile(name);
        _profiles.SaveRequested += name => _client.SaveProfile(name);
        _profiles.DeleteRequested += name => _client.DeleteProfile(name);

        _devicePanel.PriorityChanged += entries => _client.SetDevicePriority(entries);

        // Opening either side panel widens the left column, which changes how
        // narrow the mixer can go.
        _pool.SizeChanged += (_, _) => UpdateMinimumWidth();
        _devicePanel.SizeChanged += (_, _) => UpdateMinimumWidth();

        // Where the pool lives is decided in exactly one place, and it is not
        // here: see BuildHeaderRow. Until the first state arrives there is no
        // band and nothing to route, so there is nothing to show either.
        //
        // Dropping an app back on the pool stops the app being managed.
        _pool.UnassignRequested += Unassign;

        _chatMix.ValueChanged += (_, position) =>
        {
            ShowMixReadout(position);
            _client.SetChatMix(MixAcross(position));
        };

        Loaded += OnLoaded;
        Closing += OnClosing;
        Closed += OnClosed;
        Themes.Changed += Rebuild;

        // Every size change, not just the ones that cross a boundary: the
        // crossing test is cheap and lives in one place, and asking here
        // whether it matters would be a second copy of the thresholds.
        SizeChanged += (_, _) => ReflowIfNeeded();
    }

    /// <summary>
    /// Hook the DPI-change message as soon as there is a handle to hook.
    /// </summary>
    protected override void OnSourceInitialized(EventArgs e)
    {
        base.OnSourceInitialized(e);
        DpiFollow.Attach(this);
        if (_carried is { } carried)
        {
            Placement.Restore(this, carried);
        }
        // The caption bar is the desktop window manager's, not WPF's, so it has
        // to be told separately. See TitleBar.
        TitleBar.MakeDark(this);
    }

    private async void OnLoaded(object sender, RoutedEventArgs e)
    {
        Status.Text = "Connecting…";
        try
        {
            await _client.ConnectAsync();
            // Metering costs the core real work — and, for an input channel,
            // holds the microphone open — so it is asked for only while this
            // window is up, and released the moment it closes.
            _client.SubscribeMeters(true);
            Status.Text = "";
        }
        catch (Exception ex)
        {
            Status.Text = ex.Message;
            Status.Foreground = (Brush)Theme["Warn"];
        }
    }

    /// <summary>
    /// Put the window back where it was left, the first time state arrives.
    /// </summary>
    private void RestorePlacementOnce(WindowPlacement? saved)
    {
        if (_placementRestored)
        {
            return;
        }

        _placementRestored = true;

        if (saved is null)
        {
            return;
        }

        var bounds = new Placement.Bounds(
            saved.X, saved.Y, saved.Width, saved.Height, saved.Maximised);

        if (!Placement.Restore(this, bounds))
        {
            // The display it was on is gone, or the rectangle is nonsense.
            // Leaving it where WindowStartupLocation put it is the fallback;
            // saying nothing about it is deliberate, because an undocked laptop
            // is not an error.
            Status.Text = "";
        }
    }

    /// <summary>
    /// Hand the window's size and position to the core before it goes.
    /// </summary>
    /// <remarks>
    /// <c>Closing</c> rather than <c>Closed</c>: by the time <c>Closed</c> runs
    /// the handle is gone and there is nothing left to measure. The send is
    /// queued rather than awaited, and <see cref="OnClosed"/> disposes the
    /// client a moment later - see the note there.
    /// </remarks>
    private void OnClosing(object? sender, System.ComponentModel.CancelEventArgs e)
    {
        if (Placement.Read(this) is { } bounds)
        {
            _client.SetWindowPlacement(
                bounds.X, bounds.Y, bounds.Width, bounds.Height, bounds.Maximised);
        }

        // Sent after the placement, on the same connection, so the core saves
        // where the window was before it goes. OnClosed's short delay is what
        // gives both time to leave.
        if (!_closingForLanes && !_closeToTray)
        {
            _client.Quit();
        }
    }

    /// <summary>
    /// The palette changed: replace this window with one built in the new
    /// colours, exactly where this one is. See <see cref="Themes"/> for why a
    /// rebuild rather than recolouring in place.
    /// </summary>
    /// <remarks>
    /// Same process, so the core's handle on the mixer stays valid and the
    /// tray's "Open" still finds it rather than starting a second one. The
    /// replacement makes its own connection and gets full state on it, so
    /// nothing is carried across except the rectangle.
    /// </remarks>
    private void Rebuild()
    {
        var replacement = new MainWindow(Placement.Read(this));
        if (Application.Current.MainWindow == this)
        {
            Application.Current.MainWindow = replacement;
        }

        replacement.Show();
        _closingForLanes = true;
        Close();
    }

    private async void OnClosed(object? sender, EventArgs e)
    {
        Themes.Changed -= Rebuild;
        // A beat for the placement sent in OnClosing to reach the socket.
        // Disposing immediately cancels it, and the window would forget where
        // it was every single time - which is the whole feature.
        await Task.Delay(120);
        await _client.DisposeAsync();
    }

    // --- Applying state ----------------------------------------------------

    private void Apply(CoreState state)
    {
        _devices = state.Devices;
        _sessions = state.Sessions;
        _chatMixState = state.ChatMix ?? ChatMixState.Centred;

        RoutingWarning.Visibility = state.RoutingAvailable ? Visibility.Collapsed : Visibility.Visible;
        PausedBanner.Visibility = state.Settings?.Paused == true ? Visibility.Visible : Visibility.Collapsed;

        RestorePlacementOnce(state.Window);
        Themes.Choose(state.Settings?.Theme);
        _closeToTray = state.Settings?.CloseToTray ?? true;

        _channels = state.Channels;
        EnsureStrips(state.Channels);
        RebuildBandIfStale();
        UpdateStrips(state.Channels, state.Sessions);

        _override.Update(
            new ChannelState("", "", 1, 1, false, false, null, null, false, true),
            _devices);

        _chatMix.Value = MixAcross(_chatMixState.Value);
        _pool.Update(state.Sessions);
        _devicePanel.Update(state.DevicePriority);

        if (state.Profiles is { } profiles)
        {
            _profiles.Update(profiles);
        }
    }

    /// <summary>
    /// A delta carries only the sections that changed; the rest are null.
    /// </summary>
    private void ApplyDelta(Incoming delta)
    {
        if (delta.Devices is { } devices)
        {
            _devices = devices;
        }

        // Carried in deltas so a device plugged in while the list is open
        // appears in it straight away, like everything else in the window.
        if (delta.DevicePriority is { } priority)
        {
            _devicePanel.Update(priority);
        }

        // Chosen in Settings. Themes decides whether it changes anything and
        // rebuilds this window later, never from inside this handler.
        if (delta.Settings is { } settings)
        {
            Themes.Choose(settings.Theme);
            _closeToTray = settings.CloseToTray;
            PausedBanner.Visibility = settings.Paused ? Visibility.Visible : Visibility.Collapsed;
        }

        // A profile switched from the tray or a Stream Deck, or one that
        // stopped matching because a fader moved.
        if (delta.Profiles is { } profiles)
        {
            _profiles.Update(profiles);
        }

        // Changed by another client, or by switching profile.
        if (delta.ChatMix is { } mix)
        {
            _chatMixState = mix;
            RebuildBandIfStale();
            _chatMix.Value = MixAcross(mix.Value);
            UpdateMixReadout(_channels);
        }

        if (delta.Channels is { } channels)
        {
            _channels = channels;
            EnsureStrips(channels);
            RebuildBandIfStale();
            // The sessions last reported, not null. A delta that adds or
            // removes a channel rebuilds every strip, and new strips given null
            // would show no apps at all until some application happened to
            // start or stop.
            UpdateStrips(channels, _sessions);
        }

        if (delta.Sessions is { } sessions)
        {
            _sessions = sessions;
            var current = _channelOrder
                .Select(id => _strips.TryGetValue(id, out var strip) ? strip : null)
                .Where(strip => strip is not null)
                .ToList();

            foreach (var strip in current)
            {
                strip!.SetApps(sessions.Where(s => s.Channel == strip!.ChannelId).ToList());
            }

            _pool.Update(sessions);
        }
    }

    private void ApplyMeters(IReadOnlyList<MeterLevel> levels)
    {
        foreach (var level in levels)
        {
            if (_strips.TryGetValue(level.Channel, out var strip))
            {
                strip.SetMeter(level.Level);
            }
        }
    }

    /// <summary>
    /// Create strips only when the set of channels actually changes.
    /// </summary>
    /// <remarks>
    /// Compared by id and order. A volume change, a mute, a device swap — none
    /// of them reach this, which is the point: nothing the user might be
    /// holding gets rebuilt underneath them.
    /// </remarks>
    private void EnsureStrips(IReadOnlyList<ChannelState> channels)
    {
        var ids = channels.Select(c => c.Id).ToList();
        if (ids.SequenceEqual(_channelOrder))
        {
            return;
        }

        _channelOrder = ids;
        _strips.Clear();

        foreach (var channel in channels)
        {
            var strip = new ChannelStrip();

            strip.RenameRequested += (id, name) => _client.RenameChannel(id, name);
            strip.VolumeChanged += (id, value) => _client.SetChannelVolume(id, value);
            strip.MuteToggled += id => _client.ToggleChannelMute(id);
            strip.DeviceChosen += (id, deviceId) => _client.SetChannelDevice(id, deviceId);
            strip.AppDropped += (id, executables) => Assign(executables, id);
            strip.CollapseToggled += (id, collapsed) => _client.SetChannelCollapsed(id, collapsed);

            _strips[channel.Id] = strip;
        }

        ArrangeStrips();
        BuildHeaderRow(channels);
    }

    /// <summary>
    /// Tell the scaler how narrow the mixer can be laid out without clipping.
    /// </summary>
    /// <remarks>
    /// Worked out rather than measured, because WPF clamps a measured width to
    /// the space offered and so cannot report an overflow - see
    /// <see cref="FitWindow"/>. In the column layouts the minimum is every
    /// strip at its legibility floor plus the gaps between them, plus the side
    /// column in the wide layout. The stacked layout's rows have no such floor
    /// across; the window's own minimum width covers it.
    /// </remarks>
    private void UpdateMinimumWidth()
    {
        var strips = _strips.Count;
        if (_mode == LayoutMode.Stacked || strips == 0)
        {
            Fit.MinimumContentWidth = 0;
            return;
        }

        var width = strips * ChannelStrip.MinimumWidth + (strips - 1) * StripGap * 2;

        if (_mode == LayoutMode.Full)
        {
            // The side column and the 16-pixel gap that separates it.
            width += Math.Max(_pool.ActualWidth, _devicePanel.ActualWidth) + 16;
        }

        Fit.MinimumContentWidth = width;
    }

    /// <summary>
    /// Put the strips into the grid for the current layout.
    /// </summary>
    /// <remarks>
    /// Separate from <see cref="EnsureStrips"/> because the two happen for
    /// different reasons and at different rates: strips are created when the
    /// core's channel list changes, which is almost never, and arranged when
    /// the window is resized across a breakpoint. Combining them would mean a
    /// reflow destroyed and rebuilt every control in the window, and everything
    /// each one was holding.
    /// </remarks>
    private void ArrangeStrips()
    {
        StripsGrid.Children.Clear();
        StripsGrid.ColumnDefinitions.Clear();
        StripsGrid.RowDefinitions.Clear();

        var order = _channelOrder
            .Select(id => _strips.TryGetValue(id, out var strip) ? strip : null)
            .Where(strip => strip is not null)
            .Select(strip => strip!)
            .ToList();

        var stacked = _mode == LayoutMode.Stacked;

        for (var index = 0; index < order.Count; index++)
        {
            var strip = order[index];
            var first = index == 0;
            var last = index == order.Count - 1;

            strip.SetMode(_mode);

            if (stacked)
            {
                // Rows, with the gap between them rather than beside them.
                StripsGrid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
                strip.Margin = new Thickness(0, first ? 0 : StripGap * 2, 0, 0);
                Grid.SetRow(strip, index);
                Grid.SetColumn(strip, 0);
            }
            else
            {
                StripsGrid.ColumnDefinitions.Add(
                    new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
                strip.Margin = new Thickness(first ? 0 : StripGap, 0, last ? 0 : StripGap, 0);
                Grid.SetRow(strip, 0);
                Grid.SetColumn(strip, index);
            }

            StripsGrid.Children.Add(strip);
        }

        // Only the stacked layout can ever need this, and even there it is the
        // last resort rather than the plan - FitWindow shrinks the mixer to fit
        // a short window first, and gives up only at its floor.
        StripsScroll.VerticalScrollBarVisibility =
            stacked ? ScrollBarVisibility.Auto : ScrollBarVisibility.Disabled;

        // No standing reserve for a scrollbar. Keeping eight pixels clear
        // whether the bar is there or not would spend them, in the narrowest
        // layout, on something that almost never appears - and when it does,
        // the ScrollViewer takes the width from the content itself rather than
        // drawing over it.
        StripsGrid.Margin = new Thickness(0);

        UpdateMinimumWidth();
    }

    /// <summary>
    /// Re-evaluate the layout for the window's size, and reflow if it changed.
    /// </summary>
    /// <remarks>
    /// The decision itself is in <see cref="Layout.For"/>, which is given the
    /// <i>current</i> mode so that a boundary behaves as a band rather than a
    /// line. Without that, resting the window edge on a threshold flips the
    /// whole window back and forth with every pixel of mouse jitter.
    /// </remarks>
    private void ReflowIfNeeded()
    {
        var next = Layout.For(ActualWidth, ActualHeight, _mode);
        if (next == _mode)
        {
            // The band has a breakpoint of its own, inside the stacked layout,
            // so it can be crossed while the mode stays put.
            if (_channels.Count > 0 && BandNeedsTwoLines() != _bandTwoLines)
            {
                BuildHeaderRow(_channels);
            }
            return;
        }

        _mode = next;

        // The pool has no column of its own in the narrow layouts: it moves
        // into the header band as a button, so the strips get the whole width.
        var full = next == LayoutMode.Full;
        _pool.SetMode(next);
        _devicePanel.SetMode(next);

        // The window's own border. Twenty pixels is right for a desktop window
        // and is a fifth of the width of a phone-width one, where every pixel
        // of it is height taken from the channels.
        Root.Margin = new Thickness(full ? 20 : 13);

        // Release the pool before either container can claim it.
        //
        // WPF throws on a second parent rather than silently reparenting -
        // "Specified element is already the logical child of another element" -
        // and it throws from inside the SizeChanged handler, so the window does
        // not misbehave, it exits. The ContentControl has to be emptied
        // explicitly; BuildHeaderRow's own Clear covers the other direction.
        PoolColumn.Width = full ? GridLength.Auto : new GridLength(0);
        PoolHost.Visibility = full ? Visibility.Visible : Visibility.Collapsed;

        var inset = new Thickness(full ? 16 : 0, 0, 0, 0);
        StripsScroll.Margin = inset;
        HeaderRow.Margin = inset;

        ArrangeStrips();

        // Rebuilds the band, and puts the pool wherever this layout keeps it.
        BuildHeaderRow(_channels);
    }

    private void UpdateStrips(IReadOnlyList<ChannelState> channels, IReadOnlyList<SessionState>? sessions)
    {
        // Master limits every playback channel. Read once rather than per
        // strip, and defaulted to no limit if there is no Master at all.
        var master = channels.FirstOrDefault(c => c.Id == "master")?.Volume ?? 1f;

        foreach (var channel in channels)
        {
            if (!_strips.TryGetValue(channel.Id, out var strip))
            {
                continue;
            }

            var ceiling = channel.Id == "master" || channel.IsInput ? 1.0 : master;
            var mine = sessions?.Where(s => s.Channel == channel.Id).ToList();
            strip.Update(channel, _devices, mine, ceiling);
        }

        UpdateMixReadout(channels);
    }

    /// <summary>What the band was last built for: the mix's two channels and
    /// every channel's name. See <see cref="RebuildBandIfStale"/>.</summary>
    private string _bandBuiltFor = "";

    private string BandKey() =>
        $"{_chatMixState.Game}|{_chatMixState.Chat}|{string.Join("|", _channels.Select(c => c.Id + "=" + c.Name))}";

    /// <summary>
    /// Rebuild the header band when what it shows has changed underneath it:
    /// the mix now balances different channels, or one of them was renamed.
    /// </summary>
    /// <remarks>
    /// Checked on full state as well as on deltas. A command's reply is full
    /// state, so checking only deltas would miss, for example, the mix's
    /// channels being chosen from a strip's menu - the card would stay over the
    /// old channels with their old names on it.
    /// </remarks>
    private void RebuildBandIfStale()
    {
        if (_channelOrder.Count > 0 && BandKey() != _bandBuiltFor)
        {
            BuildHeaderRow(_channels);
        }
    }

    /// <summary>
    /// Build the band above the strips: status, the Game/Chat mix, the override.
    /// </summary>
    /// <remarks>
    /// <para>
    /// It uses the <i>same columns as the strips</i>, so each card sits over
    /// the channels it applies to: the mix spans the two channels it balances,
    /// and the override - which acts on all of them - takes the far end.
    /// </para>
    /// <para>
    /// The core reports which channels the mix governs rather than this
    /// assuming Game and Chat, because channels are renameable and a user may
    /// keep voice chat somewhere else. If they are not adjacent the control
    /// spans everything between them, which is the honest thing to draw.
    /// </para>
    /// </remarks>
    private void BuildHeaderRow(IReadOnlyList<ChannelState> channels)
    {
        _bandBuiltFor = BandKey();
        HeaderRow.Children.Clear();
        HeaderRow.ColumnDefinitions.Clear();

        // The cards are rebuilt from scratch, but the controls inside them are
        // long-lived fields - the mix slider carries a drag in progress, the
        // override button carries its device list. Clearing the grid removes
        // the OLD card from the grid; it does not remove these from the old
        // card, which still holds them as logical children.
        //
        // WPF then throws on the second parent rather than reparenting, from
        // inside a SizeChanged handler, so the window exits rather than
        // misbehaving. Detaching here is the one place that covers every path
        // into the band.
        Tree.Detach(Status);
        Tree.Detach(_pool);
        Tree.Detach(_devicePanel);

        // ... and in the wide layout the pool goes straight back to its own
        // column, because the band is not where it lives there.
        //
        // This is the ONLY place that decides. Deciding it here by omission and
        // undoing it later in the reflow would make the wide layout correct
        // only after a reflow had happened: a window starting wide would lose
        // "To be routed" on the first state push, with nothing to put it back.
        //
        // It sits above the early return deliberately. The detach has to be
        // unconditional - the band may be about to claim it - and everything
        // below here can exit before reaching a decision.
        if (_mode == LayoutMode.Full)
        {
            PoolHost.Content = SideColumn();
        }
        Tree.Detach(_chatMix);
        Tree.Detach(_mixReadout);
        Tree.Detach(_override);
        Tree.Detach(_profiles);

        if (channels.Count == 0)
        {
            return;
        }

        if (_mode == LayoutMode.Full)
        {
            BuildWideHeader(channels);
        }
        else
        {
            BuildNarrowHeader(channels);
        }
    }

    /// <summary>
    /// The band with one column per channel, so each card sits over what it
    /// governs.
    /// </summary>
    private void BuildWideHeader(IReadOnlyList<ChannelState> channels)
    {
        for (var index = 0; index < channels.Count; index++)
        {
            HeaderRow.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        }

        var profileColumn = ProfileColumn(channels.Count);

        PlaceOverrideAt(channels.Count - 1, channels.Count, withProfile: profileColumn is null);
        PlaceChatMix(channels);

        if (profileColumn is { } column)
        {
            PlaceProfile(column, channels.Count);
        }

        // Status goes in the first column, which is empty whenever the mix
        // control starts further right — as it does with the shipped channel
        // order — and is where the eye lands first when something is wrong.
        // It stops short of the first card, so a long message wraps rather
        // than running underneath one.
        var firstCard = new[] { MixColumns()?.First, profileColumn, channels.Count - 1 }
            .Where(c => c is not null)
            .Min() ?? 1;
        Grid.SetColumn(Status, 0);
        Grid.SetColumnSpan(Status, Math.Max(1, firstCard));
        HeaderRow.Children.Add(Status);
    }

    /// <summary>The first and last strip columns the mix control spans, if it is shown.</summary>
    private (int First, int Last)? MixColumns()
    {
        var left = _channelOrder.ToList().IndexOf(_chatMixState.Game);
        var right = _channelOrder.ToList().IndexOf(_chatMixState.Chat);
        return left < 0 || right < 0 ? null : (Math.Min(left, right), Math.Max(left, right));
    }

    /// <summary>
    /// Where the profile card goes in the wide band: the nearest free column to
    /// the left of the override card.
    /// </summary>
    /// <remarks>
    /// Next to the override because the two are the band's whole-mixer
    /// controls, and they read as a pair. With the shipped channels that is the
    /// column over Aux. Null when every column is taken - a channel order that
    /// puts the mix across everything - and the picker then shares the
    /// override's card, as it does in the narrow layouts.
    /// </remarks>
    private int? ProfileColumn(int channelCount)
    {
        var mix = MixColumns();
        for (var column = channelCount - 2; column >= 0; column--)
        {
            if (mix is not { } m || column < m.First || column > m.Last)
            {
                return column;
            }
        }
        return null;
    }

    /// <summary>"Profile": the picker in a card of its own, over one strip column.</summary>
    private void PlaceProfile(int column, int channelCount)
    {
        _profiles.ShowCaption = false;
        _profiles.Margin = new Thickness(0);
        var shell = Card("Profile", _profiles, "Switch between saved mixer setups");
        shell.Margin = StripMargin(column, column, channelCount);
        Grid.SetColumn(shell, column);
        Grid.SetColumnSpan(shell, 1);
        Grid.SetRow(shell, 0);
        HeaderRow.Children.Add(shell);
    }

    /// <summary>
    /// A labelled card in the header band, like the mix and override cards.
    /// </summary>
    private static Border Card(string label, UIElement content, string tooltip)
    {
        var layout = new StackPanel();
        layout.Children.Add(new TextBlock
        {
            Text = label,
            FontSize = (double)Theme["SizeTiny"],
            Foreground = (Brush)Theme["TextFaint"],
            Margin = new Thickness(2, 0, 0, 6),
        });
        layout.Children.Add(content);

        return new Border
        {
            Background = (Brush)Theme["Card"],
            BorderBrush = (Brush)Theme["Line"],
            BorderThickness = new Thickness(1),
            CornerRadius = new CornerRadius(14),
            Padding = new Thickness(16, 9, 16, 10),
            VerticalAlignment = VerticalAlignment.Bottom,
            ToolTip = tooltip,
            Child = layout,
        };
    }

    /// <summary>
    /// The band as three controls in a row, for the narrow layouts.
    /// </summary>
    /// <remarks>
    /// <para>
    /// <b>Column alignment is abandoned here on purpose.</b> In the wide layout
    /// each card sits over the channels it applies to, which is genuinely
    /// informative. Divide a 560-pixel window into six columns and that same
    /// idea gives the mix control about 180 pixels to hold two channel names, a
    /// slider and a readout — and in the stacked layout there are no strip
    /// columns to align to at all, because the strips are rows.
    /// </para>
    /// <para>
    /// So the band becomes what it is: the pool, the mix, and the override, in
    /// that order, with the mix taking whatever is left. The alignment was only
    /// ever worth keeping while it was true.
    /// </para>
    /// </remarks>
    private void BuildNarrowHeader(IReadOnlyList<ChannelState> channels)
    {
        // See BandNeedsTwoLines. A second line costs every strip below it the
        // height, so it is taken only when the three cards genuinely will not
        // sit beside each other.
        var twoLines = _bandTwoLines = BandNeedsTwoLines();

        HeaderRow.RowDefinitions.Clear();
        HeaderRow.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });

        if (twoLines)
        {
            HeaderRow.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });

            // Line one: the pool on the left, the override on the right.
            HeaderRow.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            HeaderRow.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

            PlaceSide(row: 0, column: 0, horizontal: twoLines);
            PlaceOverrideAt(1, row: 0, withProfile: true);

            // Line two: the mix, across both.
            PlaceChatMix(channels, column: 0, span: 2, row: 1);
        }
        else
        {
            HeaderRow.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });                     // pool
            HeaderRow.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });// mix
            HeaderRow.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });                     // override

            PlaceSide(row: 0, column: 0, horizontal: twoLines);
            PlaceChatMix(channels, column: 1, span: 1, row: 0);
            PlaceOverrideAt(2, row: 0, withProfile: true);
        }

        // The status line is empty almost always, so it overlays the band's
        // first line rather than taking a row of its own.
        Grid.SetRow(Status, 0);
        Grid.SetColumn(Status, 0);
        Grid.SetColumnSpan(Status, HeaderRow.ColumnDefinitions.Count);
        Status.VerticalAlignment = VerticalAlignment.Top;
        HeaderRow.Children.Add(Status);
    }

    /// <summary>
    /// Does the header band need a second line at this width?
    /// </summary>
    /// <remarks>
    /// <para>
    /// Three cards across a phone-width window do not fit. At 456 logical
    /// pixels the pool takes about 150 and the override about 160, which leaves
    /// the mix control 120 to hold two channel names, a slider and a live
    /// readout - not enough.
    /// </para>
    /// <para>
    /// <b>It is a question about width, not about the layout mode.</b> Stacked
    /// reaches up to 1000 pixels wide on a portrait display, where the three
    /// cards fit side by side with room to spare - and taking a second line
    /// anyway would cost about ninety pixels of height in the one layout that
    /// has none to spare.
    /// </para>
    /// <para>
    /// Sticky in the same way <see cref="Layout"/> is, and for the same reason:
    /// a boundary that is a line rather than a band flips back and forth with
    /// every pixel of jitter while the edge rests on it.
    /// </para>
    /// </remarks>
    private bool BandNeedsTwoLines() =>
        _mode == LayoutMode.Stacked
        && ActualWidth < (_bandTwoLines ? BandStacksBelow + 40 : BandStacksBelow);

    /// <summary>The width below which the band's three cards stop fitting.</summary>
    /// <remarks>
    /// The pool button and the override card are about 150 and 175 wide, and
    /// the mix needs roughly 260 before its two names, slider and readout start
    /// colliding. 620 is that sum with the margins.
    /// </remarks>
    private const double BandStacksBelow = 620;

    /// <summary>
    /// The wide layout's left column: "To be routed" above, "Audio devices"
    /// below, half the height each.
    /// </summary>
    /// <remarks>
    /// Built fresh each time rather than kept, so it cannot end up holding a
    /// panel the band has since claimed - the two panels are detached from
    /// wherever they were in <see cref="BuildHeaderRow"/> before either
    /// container is made.
    /// </remarks>
    private Grid SideColumn()
    {
        var grid = new Grid();
        grid.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        grid.RowDefinitions.Add(new RowDefinition { Height = new GridLength(StripGap * 2) });
        grid.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });

        foreach (var (panel, row) in new (FrameworkElement, int)[] { (_pool, 0), (_devicePanel, 2) })
        {
            Grid.SetRow(panel, row);
            Grid.SetColumn(panel, 0);
            Grid.SetColumnSpan(panel, 1);
            panel.HorizontalAlignment = HorizontalAlignment.Left;
            grid.Children.Add(panel);
        }

        return grid;
    }

    /// <summary>
    /// The narrow layouts: both panels as buttons, together in one band cell.
    /// </summary>
    /// <param name="horizontal">
    /// Side by side when the band has two lines and room across; stacked when
    /// the band is one line, where they are as tall as the mix card beside them
    /// and cost no height at all.
    /// </param>
    private void PlaceSide(int row, int column, bool horizontal)
    {
        var stack = new StackPanel
        {
            Orientation = horizontal ? Orientation.Horizontal : Orientation.Vertical,
            HorizontalAlignment = HorizontalAlignment.Left,
            VerticalAlignment = VerticalAlignment.Bottom,
        };

        foreach (var panel in new FrameworkElement[] { _pool, _devicePanel })
        {
            Grid.SetRow(panel, 0);
            Grid.SetColumn(panel, 0);
            panel.HorizontalAlignment = HorizontalAlignment.Left;
            stack.Children.Add(panel);
        }

        Grid.SetRow(stack, row);
        Grid.SetColumn(stack, column);
        Grid.SetColumnSpan(stack, 1);
        HeaderRow.Children.Add(stack);
    }

    /// <summary>
    /// "Audio override": send every playback channel to one device at once.
    /// </summary>
    /// <remarks>
    /// In the last column, in a card like everything else in the window, and
    /// bottom-aligned so it shares a baseline with the mix control beside it
    /// however tall either happens to be.
    /// </remarks>
    /// <param name="column">The band column the card goes in.</param>
    /// <param name="channelCount">
    /// Null in the narrow band, where the columns are the band's own three
    /// rather than one per channel, so there are no strips to line up with.
    /// </param>
    /// <param name="withProfile">
    /// Put the profile picker in this card too, under the override, with its
    /// caption inside the button. The narrow band has no column to give it: a
    /// fourth card beside the other three would leave the mix control about 85
    /// pixels at the compact layout's narrowest. Stacked in here it costs next
    /// to no height in the one-line band either, because the side column's two
    /// buttons were already the tallest thing in it.
    /// </param>
    private void PlaceOverrideAt(int column, int? channelCount = null, int row = 0, bool withProfile = false)
    {
        var last = column;

        UIElement content = _override;
        if (withProfile)
        {
            _profiles.ShowCaption = true;
            _profiles.Margin = new Thickness(0, 6, 0, 0);
            var both = new StackPanel();
            both.Children.Add(_override);
            both.Children.Add(_profiles);
            content = both;
        }

        var shell = Card("Audio override", content, "Send every playback channel to one device");
        shell.Margin = channelCount is { } count
            ? StripMargin(last, last, count)
            : new Thickness(StripGap, 0, 0, 14);

        Grid.SetColumn(shell, column);
        Grid.SetColumnSpan(shell, 1);
        Grid.SetRow(shell, row);
        HeaderRow.Children.Add(shell);
    }

    /// <summary>
    /// The margin that lines a header card up with the strips underneath it.
    /// </summary>
    /// <remarks>
    /// The strips are inset by half the gap on each inward-facing side, and a
    /// card in the same column with no margin is therefore <i>wider</i> than
    /// the strip it sits over by that much at each end. The user spotted it on
    /// the mix control: "isn't sitting perfectly in line with the Game and Chat
    /// boxes below it". Same arithmetic as <see cref="EnsureStrips"/>, and it
    /// has to stay the same arithmetic.
    /// </remarks>
    private static Thickness StripMargin(int first, int last, int count) =>
        new(first == 0 ? 0 : StripGap, 0, last == count - 1 ? 0 : StripGap, 14);

    /// <summary>Half the space between two strips.</summary>
    private const double StripGap = 7;

    private void PlaceChatMix(
        IReadOnlyList<ChannelState> channels,
        int? column = null,
        int? span = null,
        int row = 0)
    {
        var left = _channelOrder.ToList().IndexOf(_chatMixState.Game);
        var right = _channelOrder.ToList().IndexOf(_chatMixState.Chat);

        if (left < 0 || right < 0)
        {
            return;
        }

        var first = Math.Min(left, right);
        var last = Math.Max(left, right);

        var leftName = channels[first].Name;
        var rightName = channels[last].Name;

        // Which end is which, so the position can be translated to the core's
        // preference. See `_mixLeftIsGame`.
        _mixLeftIsGame = channels[first].Id == _chatMixState.Game;

        _chatMix.LeftAccent = AccentFor(channels[first].Id);
        _chatMix.RightAccent = AccentFor(channels[last].Id);

        var shell = new Border
        {
            Background = (Brush)Theme["Card"],
            BorderBrush = (Brush)Theme["Line"],
            BorderThickness = new Thickness(1),
            CornerRadius = new CornerRadius(14),
            Padding = new Thickness(16, 9, 16, 10),
            Margin = StripMargin(first, last, channels.Count),
        };

        var layout = new Grid();
        layout.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });   // labels
        layout.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });   // slider
        layout.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });   // readout

        // All three labels in ONE cell, aligned within it.
        //
        // They were in a DockPanel, with the two names docked to the sides and
        // "mix" filling what was left. That centres it in the REMAINDER, not in
        // the row — so the moment the two channel names differ in width, the
        // middle label sits off centre. Which it did, and the user spotted it.
        var labels = new Grid();
        var leftLabel = new TextBlock
        {
            Text = leftName,
            FontSize = (double)Theme["SizeTiny"],
            Foreground = (Brush)Theme["TextFaint"],
        };
        var midLabel = new TextBlock
        {
            Text = "mix",
            FontSize = (double)Theme["SizeTiny"],
            Foreground = (Brush)Theme["TextGhost"],
            HorizontalAlignment = HorizontalAlignment.Center,
        };
        var rightLabel = new TextBlock
        {
            Text = rightName,
            FontSize = (double)Theme["SizeTiny"],
            Foreground = (Brush)Theme["TextFaint"],
        };
        leftLabel.HorizontalAlignment = HorizontalAlignment.Left;
        rightLabel.HorizontalAlignment = HorizontalAlignment.Right;
        labels.Children.Add(leftLabel);
        labels.Children.Add(midLabel);
        labels.Children.Add(rightLabel);
        Grid.SetRow(labels, 0);
        layout.Children.Add(labels);

        Grid.SetRow(_chatMix, 1);
        layout.Children.Add(_chatMix);

        // What the mix is actually doing, live.
        //
        // The control expresses a ratio and the effect of a ratio is not
        // obvious from a handle position, so this shows where the two
        // channels have landed. Deliberately quiet: it is there
        // to be consulted, not read.
        _mixReadout.HorizontalAlignment = HorizontalAlignment.Center;
        _mixReadout.FontSize = (double)Theme["SizeTiny"];
        _mixReadout.Foreground = (Brush)Theme["TextGhost"];
        _mixReadout.Margin = new Thickness(0, 2, 0, 0);
        Grid.SetRow(_mixReadout, 2);
        layout.Children.Add(_mixReadout);

        shell.Child = layout;

        // The narrow band has three columns of its own, so the caller says
        // where the card goes; the wide one derives it from the channels the
        // mix actually governs.
        Grid.SetColumn(shell, column ?? first);
        Grid.SetColumnSpan(shell, span ?? (last - first + 1));
        Grid.SetRow(shell, row);
        if (column is not null)
        {
            shell.Margin = new Thickness(0, 0, 0, 14);
        }

        HeaderRow.Children.Add(shell);
    }

    /// <summary>Remember the two mixed channels, then redraw the readout.</summary>
    private void UpdateMixReadout(IReadOnlyList<ChannelState> channels)
    {
        // Left and right as drawn, which follows the strips' order and not the
        // core's naming. Taking `game` as the left assumed the two agreed, so
        // once the mix could balance any two channels the readout named them
        // the wrong way round and put the quietening on the wrong one.
        var game = channels.FirstOrDefault(c => c.Id == _chatMixState.Game);
        var chat = channels.FirstOrDefault(c => c.Id == _chatMixState.Chat);
        var (left, right) = _mixLeftIsGame ? (game, chat) : (chat, game);

        if (left is null || right is null)
        {
            _mixReadout.Text = "";
            return;
        }

        _mixLeft = (left.Name, left.EffectiveVolume);
        _mixRight = (right.Name, right.EffectiveVolume);
        ShowMixReadout(MixAcross(_chatMixState.Value));
    }

    /// <summary>
    /// What each channel comes out at for a given slider position.
    /// </summary>
    /// <remarks>
    /// The same arithmetic the core applies, repeated here so the number can
    /// follow the handle instead of waiting for a round trip. It is a display
    /// of a rule, not a second copy of it: the core remains the only thing that
    /// sets a volume, and <c>config::ChatMix</c> is where the rule lives.
    ///
    /// <b>In slider terms, not the core's:</b> pulling toward a label makes
    /// that channel the loud one and quietens the other by `1 - |position|`.
    /// Centre leaves both untouched.
    /// </remarks>
    private void ShowMixReadout(double position)
    {
        if (_mixLeft.Name.Length == 0)
        {
            return;
        }

        var toward = Math.Clamp(position, -1, 1);

        // Pulled left favours the left channel, so it is the RIGHT one that
        // gets quieter. A readout derived from the same wrong assumption as the
        // control would agree with a bug instead of exposing it, so this is
        // worked out independently of the slider's own conversion.
        var leftFactor = toward > 0 ? 1.0 - Math.Abs(toward) : 1.0;
        var rightFactor = toward < 0 ? 1.0 - Math.Abs(toward) : 1.0;

        var leftPercent = Math.Round(_mixLeft.Level * leftFactor * 100);
        var rightPercent = Math.Round(_mixRight.Level * rightFactor * 100);

        // A middle dot rather than a run of spaces. Spaces set the gap by the
        // width of the font's space glyph, which is a guess that changes with
        // the font; a separator says the two readings are one line.
        _mixReadout.Text =
            $"{_mixLeft.Name} {leftPercent:0}%   ·   {_mixRight.Name} {rightPercent:0}%";
    }

    private void OverrideAll(string deviceId)
    {
        // Written per channel rather than as a global routing mode, so that
        // afterwards any single channel can still be changed on its own and
        // nothing has to be un-overridden first.
        foreach (var (id, strip) in _strips)
        {
            if (!strip.IsInput)
            {
                _client.SetChannelDevice(id, deviceId);
            }
        }
    }


    /// <summary>
    /// The application's resources, not this window's.
    /// </summary>
    /// <remarks>
    /// <c>Window.Resources["key"]</c> does <b>not</b> walk up to the
    /// application's dictionary — the indexer only looks in the dictionary it is
    /// called on, and returns null for anything else. That is why the theme is
    /// reached through <see cref="Application.Current"/> here while the XAML can
    /// use <c>{StaticResource}</c> freely: XAML lookup does inherit.
    ///
    /// Worth a named helper rather than repeating it, because the failure is a
    /// null unboxed into a double at startup, which says nothing about the cause.
    /// </remarks>
    private static ResourceDictionary Theme => Application.Current.Resources;

    // --- IAppCommands: what an app chip's menu can ask for ------------------

    IReadOnlyList<(string Id, string Name)> IAppCommands.Channels =>
        _channels.Where(c => !c.IsInput && c.Id != "master").Select(c => (c.Id, c.Name)).ToList();

    void IAppCommands.Assign(string executable, string channel) => Assign([executable], channel);
    void IAppCommands.Unassign(string executable) => Unassign([executable]);

    (string A, string B) IChannelCommands.MixPair => (_chatMixState.Game, _chatMixState.Chat);

    IReadOnlyList<(string Id, string Name)> IChannelCommands.MixCandidates =>
        ((IAppCommands)this).Channels;

    void IChannelCommands.SetMixPair(string a, string b) => _client.SetChatMixChannels(a, b);

    void IChannelCommands.Remove(string channel) => _client.RemoveChannel(channel);

    // --- Moving apps, and saying so --------------------------------------------

    /// <summary>Put these apps in a channel, and confirm the rules made.</summary>
    private void Assign(IReadOnlyList<string> executables, string channel)
    {
        // Only the ones not already there: dropping a chip back where it came
        // from makes no rule and deserves no message.
        var moving = executables.Where(e => ChannelOf(e) != channel).ToList();

        foreach (var executable in moving)
        {
            _client.AssignApp(executable, channel);
        }

        if (moving.Count > 0)
        {
            var name = _channels.FirstOrDefault(c => c.Id == channel)?.Name ?? channel;
            Confirm($"{Several(moving)}  \u2192  {name}");
        }
    }

    /// <summary>Take these apps out of their channels.</summary>
    private void Unassign(IReadOnlyList<string> executables)
    {
        var moving = executables.Where(e => ChannelOf(e) is not null).ToList();

        foreach (var executable in moving)
        {
            _client.UnassignApp(executable);
        }

        if (moving.Count > 0)
        {
            Confirm($"{Several(moving)}  \u2192  To be routed");
        }
    }

    private string? ChannelOf(string executable) =>
        _sessions.FirstOrDefault(s => s.Executable.Equals(executable, StringComparison.OrdinalIgnoreCase))?.Channel;

    private static string Several(IReadOnlyList<string> executables) => executables.Count switch
    {
        1 => executables[0],
        2 => $"{executables[0]} and {executables[1]}",
        _ => $"{executables.Count} apps",
    };

    /// <summary>Show a line at the foot of the window for a couple of seconds.</summary>
    private void Confirm(string text, bool warn = false)
    {
        _toastText.Text = text;
        _toastText.Foreground = (Brush)Theme[warn ? "Warn" : "Text"];
        _toast.BeginAnimation(OpacityProperty, null);
        _toast.Opacity = 1;
        _toastTimer.Stop();
        _toastTimer.Start();
    }

    private static bool InsideChip(DependencyObject? element)
    {
        while (element is not null)
        {
            if (element is AppChip)
            {
                return true;
            }

            element = element is Visual
                ? VisualTreeHelper.GetParent(element)
                : LogicalTreeHelper.GetParent(element);
        }

        return false;
    }
    void IAppCommands.Ignore(string executable) => _client.IgnoreApp(executable);
    void IAppCommands.SetTrim(string executable, double? trim) => _client.SetAppTrim(executable, trim);

    private static Brush AccentFor(string channelId)
    {
        var key = "Accent." + channelId;
        return Application.Current.Resources.Contains(key)
            ? (Brush)Application.Current.Resources[key]
            : (Brush)Application.Current.Resources["AccentFallback"];
    }
}
