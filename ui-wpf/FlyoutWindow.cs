using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;

using Lanes.Ui.Api;
using Lanes.Ui.Controls;

namespace Lanes.Ui;

/// <summary>
/// The tray's quick mixer: every channel's fader and mute, and nothing else.
/// </summary>
/// <remarks>
/// <para>
/// A left click on the tray icon opens it: a small panel with just channel
/// faders and mutes. It is this executable run with
/// <c>--flyout</c>, so it is a pure client of the core exactly as the mixer is,
/// and it is torn down when it closes - the process exits and its memory goes
/// back - rather than hidden.
/// </para>
/// <para>
/// <b>It closes itself when it loses focus</b>, like every flyout Windows
/// draws from the notification area: click anywhere else and it is gone. Esc
/// closes it too. Clicking the tray icon again while it is open closes it
/// rather than reopening it, which is handled in the core (see
/// <c>lifecycle::open_flyout</c>), because by the time that click arrives the
/// flyout has already closed itself on losing focus.
/// </para>
/// <para>
/// <b>The strips are the mixer's own</b>, in their stacked-row form with
/// <see cref="ChannelStrip.Minimal"/> set, so a fader here behaves exactly as
/// one in the mixer does. Two implementations of a fader would be two sets of
/// drag, settle and detent behaviour to keep in step.
/// </para>
/// </remarks>
public sealed class FlyoutWindow : Window
{
    /// <summary>Wide enough for a usable horizontal fader, narrow enough to be a flyout.</summary>
    private const double PanelWidth = 360;

    private readonly CoreClient _client;
    private readonly StackPanel _rows = new();
    private readonly Dictionary<string, ChannelStrip> _strips = [];
    private readonly TextBlock _status = new();
    private IReadOnlyList<string> _order = [];

    /// <summary>Whether the flyout has ever had focus.</summary>
    /// <remarks>
    /// Closing on deactivation is only right once it has been active. A window
    /// that never received focus in the first place - Windows refuses focus to
    /// a process it does not trust with it - would otherwise never close, or,
    /// worse, close the instant it appeared.
    /// </remarks>
    private bool _wasActive;

    /// <summary>Set once closing has begun. See <see cref="CloseOnce"/>.</summary>
    private bool _closing;

    /// <summary>Close, unless closing already - the one way this window closes itself.</summary>
    private void CloseOnce()
    {
        if (!_closing)
        {
            _closing = true;
            Close();
        }
    }

    public FlyoutWindow()
    {
        Title = Api.CoreProcess.Titled("Lanes quick mixer");
        WindowStyle = WindowStyle.None;
        ResizeMode = ResizeMode.NoResize;
        ShowInTaskbar = false;
        Topmost = true;
        SizeToContent = SizeToContent.Height;
        Width = PanelWidth;
        Background = Res("Ground");
        FontFamily = (FontFamily)Application.Current.Resources["UiFont"];
        UseLayoutRounding = true;
        SnapsToDevicePixels = true;
        TextOptions.SetTextFormattingMode(this, TextFormattingMode.Ideal);

        // Off-screen until it has a size, so it never flashes up in the middle
        // of the display before being moved beside the tray.
        WindowStartupLocation = WindowStartupLocation.Manual;
        Left = -10000;
        Top = -10000;

        Content = BuildContent();

        _client = new CoreClient(Dispatcher);
        _client.StateReceived += Apply;
        _client.DeltaReceived += delta =>
        {
            if (delta.Channels is { } channels)
            {
                ApplyChannels(channels);
            }
        };
        _client.MetersReceived += levels =>
        {
            foreach (var level in levels)
            {
                if (_strips.TryGetValue(level.Channel, out var strip))
                {
                    strip.SetMeter(level.Level);
                }
            }
        };
        _client.Connected += () => _client.SubscribeMeters(true);
        _client.Disconnected += why => _status.Text = $"Reconnecting… ({why})";
        _client.ShuttingDown += CloseOnce;

        Loaded += async (_, _) =>
        {
            PlaceBesideTray();
            Activate();
            try
            {
                await _client.ConnectAsync();
                _client.SubscribeMeters(true);
                _status.Text = "";
            }
            catch (Exception ex)
            {
                _status.Text = ex.Message;
            }
        };

        // Re-placed whenever the height changes - the first state push adds
        // the rows - so the bottom edge stays anchored above the taskbar.
        //
        // Deferred, not done in the handler. WPF raises SizeChanged before the
        // native window has been resized to match, so the rectangle Windows
        // reports at that moment is the OLD one: a ~90-pixel window would be
        // anchored above the taskbar and then grow 800 pixels downwards, off
        // the bottom of the screen. At Background priority the resize has
        // happened.
        SizeChanged += (_, _) => Dispatcher.BeginInvoke(
            PlaceBesideTray, System.Windows.Threading.DispatcherPriority.Background);

        Activated += (_, _) => _wasActive = true;
        Deactivated += (_, _) =>
        {
            if (_wasActive)
            {
                CloseOnce();
            }
        };

        // Closing makes the window lose focus, which raises Deactivated, which
        // would ask to close again - and WPF throws on a Close() during a
        // close. Without this guard every way of closing except clicking
        // elsewhere would crash the process: Esc, "Open mixer", the core
        // shutting down.
        Closing += (_, _) => _closing = true;

        KeyDown += (_, e) =>
        {
            if (e.Key == Key.Escape)
            {
                CloseOnce();
            }
        };

        // A beat for a command sent on the way out - "Open mixer" - to reach the
        // socket before it is disposed. The mixer does the same, for the same
        // reason: disposing at once cancels the send.
        Closed += async (_, _) =>
        {
            await Task.Delay(120);
            await _client.DisposeAsync();
        };
    }

    private Border BuildContent()
    {
        var title = new TextBlock
        {
            Text = "Lanes",
            FontSize = (double)Application.Current.Resources["SizeBody"],
            FontWeight = FontWeights.SemiBold,
            Foreground = Res("Text"),
            VerticalAlignment = VerticalAlignment.Center,
        };

        var open = new Border
        {
            Padding = new Thickness(10, 5, 10, 5),
            CornerRadius = new CornerRadius(8),
            Background = Res("Raised"),
            Cursor = Cursors.Hand,
            ToolTip = "Open the full mixer",
            Child = new TextBlock
            {
                Text = "Open mixer",
                FontSize = (double)Application.Current.Resources["SizeSmall"],
                Foreground = Res("TextDim"),
            },
        };
        open.MouseEnter += (_, _) => open.Opacity = 0.8;
        open.MouseLeave += (_, _) => open.Opacity = 1.0;
        open.MouseLeftButtonUp += (_, _) =>
        {
            // The core opens it - it is the one place that knows whether the
            // mixer is already open - and this closes, since the mixer is what
            // the user wants now.
            _client.ShowWindow();
            CloseOnce();
        };
        DockPanel.SetDock(open, Dock.Right);

        var header = new DockPanel { LastChildFill = true, Margin = new Thickness(4, 0, 0, 10) };
        header.Children.Add(open);
        header.Children.Add(title);

        _status.FontSize = (double)Application.Current.Resources["SizeTiny"];
        _status.Foreground = Res("Warn");
        _status.TextWrapping = TextWrapping.Wrap;
        _status.Margin = new Thickness(4, 0, 0, 8);
        _status.Text = "Connecting…";

        var layout = new StackPanel();
        layout.Children.Add(header);
        layout.Children.Add(_status);
        layout.Children.Add(_rows);

        return new Border
        {
            BorderBrush = Res("LineStrong"),
            BorderThickness = new Thickness(1),
            Padding = new Thickness(12),
            Child = layout,
        };
    }

    private void Apply(CoreState state) => ApplyChannels(state.Channels);

    /// <summary>Create rows when the set of channels changes; otherwise update in place.</summary>
    /// <remarks>
    /// The same rule the mixer is built around: a row that is rebuilt while it
    /// is being dragged loses the drag.
    /// </remarks>
    private void ApplyChannels(IReadOnlyList<ChannelState> channels)
    {
        var order = channels.Select(c => c.Id).ToList();
        if (!order.SequenceEqual(_order))
        {
            _order = order;
            _rows.Children.Clear();
            _strips.Clear();

            foreach (var channel in channels)
            {
                var strip = new ChannelStrip { Minimal = true, Margin = new Thickness(0, 0, 0, 8) };
                strip.SetMode(LayoutMode.Stacked);
                strip.VolumeChanged += (id, value) => _client.SetChannelVolume(id, value);
                strip.MuteToggled += id => _client.ToggleChannelMute(id);
                _strips[channel.Id] = strip;
                _rows.Children.Add(strip);
            }

            if (_rows.Children.Count > 0 && _rows.Children[^1] is FrameworkElement last)
            {
                last.Margin = new Thickness(0);
            }
        }

        // Master limits every playback channel; see the mixer's UpdateStrips.
        var master = channels.FirstOrDefault(c => c.Id == "master")?.Volume ?? 1f;
        foreach (var channel in channels)
        {
            if (_strips.TryGetValue(channel.Id, out var strip))
            {
                var ceiling = channel.Id == "master" || channel.IsInput ? 1.0 : master;
                strip.Update(channel, [], null, ceiling);
            }
        }
    }

    /// <summary>Beside the tray: the corner of the pointer's monitor nearest the pointer.</summary>
    private void PlaceBesideTray() => Corner.Place(this, nearPointer: true);

    private static Brush Res(string key) => (Brush)Application.Current.Resources[key];
}
