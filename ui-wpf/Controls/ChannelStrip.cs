using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;

using Lanes.Ui.Api;

using Rectangle = System.Windows.Shapes.Rectangle;

namespace Lanes.Ui.Controls;

/// <summary>
/// One channel: name, fader with its meter, mute, output device, app count.
/// </summary>
/// <remarks>
/// <para>
/// <b>Built once, then updated in place.</b> The core echoes state after every
/// command, so a list of strips replaced wholesale on every state push would
/// destroy and recreate the very control being dragged. Nothing here is ever
/// recreated in response to a value changing — <see cref="Update"/> sets
/// properties on existing controls and returns.
/// </para>
/// <para>
/// Worth stating plainly because the same trap exists in every retained-mode
/// framework, and rebuilding a list on each update is the obvious thing to do.
/// </para>
/// </remarks>
public sealed class ChannelStrip : Border
{
    /// <summary>
    /// The channel's colour, as a lit top edge falling into the strip.
    /// </summary>
    /// <remarks>
    /// <para>
    /// Full width, and fading into the body of the card, so the accent belongs
    /// to the whole strip rather than to the name - which is what lets the name
    /// be centred and gives the column a header.
    /// </para>
    /// <para>
    /// <b>It does not take any layout space.</b> It shares a cell with the
    /// strip's contents and is drawn first, so the channel name sits on the
    /// wash rather than below it. A 36-pixel band in a row of its own would
    /// have pushed everything down by 36 pixels to show a gradient.
    /// </para>
    /// </remarks>
    private readonly Rectangle _accentRule = new() { Height = HeaderFade };

    /// <summary>How far the channel's colour reaches into the card.</summary>
    /// <remarks>
    /// Far enough to sit behind the channel name and stop before the readout,
    /// so it reads as a header rather than as a tint over the whole strip.
    /// </remarks>
    private const double HeaderFade = 36;

    private readonly TextBlock _name = new();
    private readonly TextBlock _readout = new();
    private readonly Fader _fader = new();
    private readonly Border _mute;
    private readonly Grid _muteIcon;
    private readonly DeviceButton _device = new();

    /// <summary>The clickable "3 apps" row.</summary>
    private readonly Border _appsHeader;
    private readonly TextBlock _appsCount = new();
    private readonly System.Windows.Shapes.Path _appsChevron;

    /// <summary>The chips, hidden until the row is opened.</summary>
    private readonly StackPanel _chips = new();

    /// <summary>What <see cref="_chips"/> currently holds, in order.</summary>
    /// <remarks>
    /// Rebuilt only when this changes. A session's "playing" flag flips
    /// constantly and rebuilding on it would destroy a chip mid-drag.
    /// </remarks>
    private IReadOnlyList<string> _chipOrder = [];

    private bool _collapsed = true;

    /// <summary>The padded grid the contents are placed in.</summary>
    private readonly Grid _layout = new() { Margin = new Thickness(14, 13, 14, 16) };

    /// <summary>Mute and device, kept together so they move as a pair.</summary>
    private readonly Grid _controls = new();

    /// <summary>Name and readout, which travel together in every layout.</summary>
    private readonly StackPanel _heading = new();

    private LayoutMode _mode = LayoutMode.Full;

    /// <summary>
    /// Holds the fader in the stacked layout, and tells it how long to be.
    /// </summary>
    /// <remarks>
    /// A rotated element cannot simply be told to stretch. WPF measures a
    /// <c>LayoutTransform</c>ed child against the inverse of its own transform,
    /// so a <c>Stretch</c> alignment resolves against a constraint the child
    /// cannot see the width of, and the fader settles at its minimum length
    /// instead of filling the row - which is exactly what it did.
    ///
    /// Giving it a definite local height taken from this host's actual width is
    /// the direct way to say what was meant: the fader's length is the row's
    /// width.
    /// </remarks>
    private readonly Grid _faderHost = new();

    /// <summary>What <see cref="_accentRule"/> was last painted for.</summary>
    /// <remarks>
    /// State arrives about thirty times a second while meters are running, and
    /// the fading brush is built rather than looked up. Cheap, but there is no
    /// reason to build six of them per push when nothing has changed.
    /// </remarks>
    private (Brush? Accent, bool Muted) _rulePainted;

    public string ChannelId { get; private set; } = "";
    public bool IsInput { get; private set; }

    private bool _minimal;

    /// <summary>
    /// Just the name, the fader and the mute: no device button, no apps.
    /// </summary>
    /// <remarks>
    /// For the tray's quick mixer: a small panel with just channel faders and
    /// mutes. It is the same strip
    /// rather than a second control, so the fader behaves identically in both
    /// places - the drag rationing, the settle, the detents, the ceiling - and
    /// there is one of each to fix. Set it before the first
    /// <see cref="SetMode"/>.
    /// </remarks>
    public bool Minimal
    {
        get => _minimal;
        set
        {
            _minimal = value;
            _device.Visibility = value ? Visibility.Collapsed : Visibility.Visible;
            ShowMoreButton();
        }
    }

    public event Action<string, double>? VolumeChanged;
    public event Action<string>? MuteToggled;
    public event Action<string, string>? DeviceChosen;

    /// <summary>The apps list was opened or closed. Persisted by the core.</summary>
    public event Action<string, bool>? CollapseToggled;

    /// <summary>One or more app chips were dropped here: the channel id and the
    /// executables. Creates a persistent rule for each.</summary>
    public event Action<string, IReadOnlyList<string>>? AppDropped;

    /// <summary>The user renamed the channel. Carries the id and the new name.</summary>
    public event Action<string, string>? RenameRequested;

    /// <summary>The name being edited, while it is.</summary>
    private TextBox? _editor;

    /// <summary>What the channel menu can do. Set once by the window.</summary>
    /// <remarks>Static for the same reason as <see cref="AppChip.Commands"/>.</remarks>
    public static IChannelCommands? Commands { get; set; }

    /// <summary>
    /// The channel's menu button: three dots in the top corner, shown only
    /// while the pointer is over the strip.
    /// </summary>
    /// <remarks>
    /// Hidden until wanted, because the window's rule is that nothing is on
    /// show until it is asked for, and six menu buttons permanently in six
    /// headers would be exactly the noise the design avoids. Right-clicking the name opens the same menu, which is
    /// the only way in where there is no room for the button - the stacked
    /// rows - and in the quick mixer there is neither.
    /// </remarks>
    private readonly Border _more;

    private bool _removable;
    private string _channelName = "";

    public ChannelStrip()
    {
        BorderBrush = (Brush)Application.Current.Resources["Line"];
        BorderThickness = new Thickness(1);
        CornerRadius = new CornerRadius(14);
        Background = (Brush)Application.Current.Resources["Card"];
        // The floor is a legibility floor, and an honest one: this is a width
        // the strip can genuinely render at.
        MinWidth = MinimumWidth;
        SnapsToDevicePixels = true;

        // One cell, two layers. The accent wash is added first so it is drawn
        // behind the contents and claims no height of its own; the padded
        // layout sits on top of it. It has to reach the card's full width to
        // read as a header, which is why it is not inside the padded grid.
        var shell = new Grid();

        _accentRule.HorizontalAlignment = HorizontalAlignment.Stretch;
        _accentRule.VerticalAlignment = VerticalAlignment.Top;
        _accentRule.IsHitTestVisible = false;
        shell.Children.Add(_accentRule);

        shell.Children.Add(_layout);

        var dots = new StackPanel { Orientation = Orientation.Horizontal, VerticalAlignment = VerticalAlignment.Center, HorizontalAlignment = HorizontalAlignment.Center };
        for (var i = 0; i < 3; i++)
        {
            dots.Children.Add(new System.Windows.Shapes.Ellipse
            {
                Width = 3.2,
                Height = 3.2,
                Margin = new Thickness(1.4, 0, 1.4, 0),
                Fill = (Brush)Application.Current.Resources["TextDim"],
            });
        }
        _more = new Border
        {
            Width = 26,
            Height = 20,
            CornerRadius = new CornerRadius(6),
            HorizontalAlignment = HorizontalAlignment.Right,
            VerticalAlignment = VerticalAlignment.Top,
            Margin = new Thickness(0, 9, 9, 0),
            Background = Brushes.Transparent,
            Cursor = Cursors.Hand,
            ToolTip = "Channel options",
            Opacity = 0,
            Child = dots,
        };
        _more.MouseEnter += (_, _) => _more.Background = (Brush)Application.Current.Resources["Hover"];
        _more.MouseLeave += (_, _) => _more.Background = Brushes.Transparent;
        _more.MouseLeftButtonUp += (_, e) =>
        {
            e.Handled = true;
            OpenMenu(_more);
        };
        shell.Children.Add(_more);

        MouseEnter += (_, _) => _more.Opacity = 1;
        MouseLeave += (_, _) => _more.Opacity = 0;

        // Clip the contents to the card's own rounded rectangle.
        //
        // A Border does not do this: its corner radius shapes the border and
        // the background it paints, and children are free to draw outside it.
        // Without the clip the accent rule ran straight across the top while
        // the corners curved away underneath, so it read as a line floating
        // above the strip rather than as the strip's own header.
        //
        // The radius is the border's less its thickness, which is the inner
        // curve - using the outer one leaves a hairline of card showing
        // outside the clip at each corner.
        shell.SizeChanged += (_, e) =>
        {
            var radius = Math.Max(0, CornerRadius.TopLeft - BorderThickness.Top);
            shell.Clip = new RectangleGeometry(
                new Rect(0, 0, e.NewSize.Width, e.NewSize.Height), radius, radius);
        };
        // --- Header: the channel name is the loudest text on screen ---------
        //
        // Centred under the accent rule. The two go together: a name pushed to the left needs something beside it to
        // balance against, and the rule is now doing that across the whole
        // strip instead.
        _name.FontSize = (double)Application.Current.Resources["SizeChannel"];
        _name.FontWeight = FontWeights.SemiBold;
        _name.HorizontalAlignment = HorizontalAlignment.Center;
        _name.TextTrimming = TextTrimming.CharacterEllipsis;
        _heading.Children.Add(_name);

        // Double-click to rename. Not in the quick mixer, which is for moving faders and closes the moment focus goes.
        _name.MouseLeftButtonDown += (_, e) =>
        {
            if (e.ClickCount == 2 && !_minimal)
            {
                e.Handled = true;
                BeginRename();
            }
        };
        _name.MouseRightButtonUp += (_, e) =>
        {
            if (!_minimal)
            {
                e.Handled = true;
                OpenMenu(_name);
            }
        };

        // --- Readout --------------------------------------------------------
        _readout.HorizontalAlignment = HorizontalAlignment.Center;
        _readout.Margin = new Thickness(0, 4, 0, 14);
        _readout.FontSize = (double)Application.Current.Resources["SizeSmall"];
        _readout.Foreground = (Brush)Application.Current.Resources["TextDim"];
        _heading.Children.Add(_readout);

        // --- Fader ----------------------------------------------------------
        // Wide enough for the detent paddles to sit clear of the cap: the
        // paddle heads run down the left, the track stays centred, and the cap
        // needs room either side of it without landing on a head.
        //
        // 76 was not enough. The gap between the end of a paddle's taper and
        // the edge of the cap was three pixels, so whenever the channel sat on
        // a quarter - which is most of the time, since that is what detents are
        // for - the two touched and merged into one shape.
        _fader.Width = FaderThickness;
        _fader.MinHeight = 90;
        _fader.HorizontalAlignment = HorizontalAlignment.Center;
        _fader.ValueChanged += (_, value) =>
        {
            // Updated here, not from the echo, so the number tracks the handle.
            _readout.Text = _fader.DisplayText;
            VolumeChanged?.Invoke(ChannelId, value);
        };

        // --- Control row: exactly one mute per strip, and it lives here ------
        _controls.Height = 34;
        _controls.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(34) });
        _controls.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(7) });
        _controls.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });

        _muteIcon = Icons.Speaker(false, (Brush)Application.Current.Resources["TextDim"], 15);
        _muteIcon.HorizontalAlignment = HorizontalAlignment.Center;
        _muteIcon.VerticalAlignment = VerticalAlignment.Center;

        _mute = new Border
        {
            CornerRadius = new CornerRadius(10),
            Background = (Brush)Application.Current.Resources["Raised"],
            Cursor = Cursors.Hand,
            Child = _muteIcon,
        };
        _mute.MouseLeftButtonUp += (_, _) => MuteToggled?.Invoke(ChannelId);

        // A control with no hover state reads as a label until it is clicked.
        _mute.MouseEnter += (_, _) => _mute.Opacity = 0.78;
        _mute.MouseLeave += (_, _) => _mute.Opacity = 1.0;
        Grid.SetColumn(_mute, 0);
        _controls.Children.Add(_mute);

        _device.DeviceChosen += id => DeviceChosen?.Invoke(ChannelId, id);
        Grid.SetColumn(_device, 2);
        _controls.Children.Add(_device);

        // --- Apps -----------------------------------------------------------
        //
        // Collapsed to a single row carrying a count, and it opens only when
        // the user opens it. That is the window's standing rule, and the same
        // chevron-and-count idiom the "To be routed" rail uses, so there is one
        // collapse pattern here rather than two.
        //
        // With no applications there is no chevron and no hover: a label that
        // says "0 apps" and looks clickable would advertise something to see
        // and then refuse to show it.
        _appsChevron = Icons.Chevron(
            Icons.Direction.Down, (Brush)Application.Current.Resources["TextGhost"], 9);
        _appsChevron.VerticalAlignment = VerticalAlignment.Center;
        _appsChevron.Margin = new Thickness(0, 0, 6, 0);

        _appsCount.FontSize = (double)Application.Current.Resources["SizeTiny"];
        _appsCount.Foreground = (Brush)Application.Current.Resources["TextGhost"];
        _appsCount.VerticalAlignment = VerticalAlignment.Center;

        var headerRow = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            HorizontalAlignment = HorizontalAlignment.Center,
        };
        headerRow.Children.Add(_appsChevron);
        headerRow.Children.Add(_appsCount);

        _appsHeader = new Border
        {
            Background = Brushes.Transparent,
            CornerRadius = new CornerRadius(7),
            Padding = new Thickness(6, 4, 6, 4),
            Cursor = Cursors.Hand,
            Child = headerRow,
        };
        _appsHeader.MouseLeftButtonUp += (_, _) => CollapseToggled?.Invoke(ChannelId, !_collapsed);
        _appsHeader.MouseEnter += (_, _) =>
            _appsHeader.Background = (Brush)Application.Current.Resources["Hover"];
        _appsHeader.MouseLeave += (_, _) => _appsHeader.Background = Brushes.Transparent;

        _chips.Visibility = Visibility.Collapsed;

        // The header and the chips are placed separately rather than wrapped in
        // a panel of their own, because the two layouts do not keep them
        // together: a column strip stacks them under the control row, and a row
        // puts the header up on the first line with the chips still below.
        Child = shell;

        // The fader's length in the stacked layout is this host's width. See
        // the field's note for why stretching does not do it.
        _faderHost.SizeChanged += (_, e) =>
        {
            if (_mode == LayoutMode.Stacked)
            {
                _fader.Height = Math.Max(90, e.NewSize.Width);
            }
        };

        // Place everything for the default layout. Nothing below this point
        // creates a control; SetMode only ever moves the ones built above.
        SetMode(LayoutMode.Full);

        // --- Drop target ----------------------------------------------------
        //
        // A collapsed or empty strip is still a valid target, and it has to LOOK
        // like one during a drag, so the valid zone is obvious.
        AllowDrop = true;

        DragOver += (_, e) =>
        {
            var welcome = !IsInput && e.Data.GetDataPresent(DataFormats.StringFormat);
            e.Effects = welcome ? DragDropEffects.Move : DragDropEffects.None;
            if (welcome)
            {
                Background = AccentFor(ChannelId).Clone();
                Background.Opacity = 0.06;
                BorderBrush = AccentFor(ChannelId);
            }
            e.Handled = true;
        };

        DragLeave += (_, _) => ClearDropHighlight();

        Drop += (_, e) =>
        {
            ClearDropHighlight();
            if (!IsInput && AppChip.Dragged(e.Data) is { Count: > 0 } executables)
            {
                AppDropped?.Invoke(ChannelId, executables);
            }
            e.Handled = true;
        };
    }

    /// <summary>
    /// The narrowest a strip can be and still be read. Public because the
    /// window works out its own minimum width from it.
    /// </summary>
    public const double MinimumWidth = 110;

    /// <summary>How wide the fader is across the track. Its length is the layout's.</summary>
    private const double FaderThickness = 88;

    /// <summary>
    /// Arrange for a layout. Moves the controls; never builds one.
    /// </summary>
    /// <remarks>
    /// <para>
    /// <b>This is not a contradiction of the "never rebuild" rule.</b> That
    /// rule is about <i>state</i>: a volume arriving from the core must not
    /// destroy the fader somebody is dragging. A layout change is a different
    /// event entirely — it happens when the window is resized, which the user
    /// is doing with the window edge rather than with a control, and it happens
    /// once per crossing rather than thirty times a second.
    /// </para>
    /// <para>
    /// The controls themselves are the same objects throughout. Only their
    /// parent and their grid position change, which is why a channel keeps its
    /// meter, its settled fader value and its open apps list across a
    /// reflow.
    /// </para>
    /// </remarks>
    public void SetMode(LayoutMode mode)
    {
        if (mode == _mode && _layout.Children.Count > 0)
        {
            return;
        }

        _mode = mode;
        ShowMoreButton();

        // A child may have exactly one parent, so everything comes out before
        // anything goes back in.
        _layout.Children.Clear();
        _layout.RowDefinitions.Clear();
        _layout.ColumnDefinitions.Clear();

        if (mode == LayoutMode.Stacked)
        {
            ArrangeAsRow();
        }
        else
        {
            ArrangeAsColumn(mode);
        }
    }

    /// <summary>The desktop shape: a column, fader taking the vertical space.</summary>
    private void ArrangeAsColumn(LayoutMode mode)
    {
        _layout.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });                     // heading
        _layout.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });// fader
        _layout.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });                     // controls
        _layout.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });                     // apps header
        _layout.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });                     // chips

        _heading.Orientation = Orientation.Vertical;
        _heading.HorizontalAlignment = HorizontalAlignment.Stretch;
        _heading.Margin = new Thickness(0);
        _name.HorizontalAlignment = HorizontalAlignment.Center;
        _name.VerticalAlignment = VerticalAlignment.Top;
        _readout.HorizontalAlignment = HorizontalAlignment.Center;
        _readout.VerticalAlignment = VerticalAlignment.Top;
        _readout.Margin = new Thickness(0, 4, 0, 14);

        _fader.LayoutTransform = Transform.Identity;
        _fader.VerticalAlignment = VerticalAlignment.Stretch;
        _fader.HorizontalAlignment = HorizontalAlignment.Center;
        _fader.Margin = new Thickness(0, 0, 0, 14);
        _fader.Height = double.NaN;

        _accentRule.Height = HeaderFade;
        _layout.Margin = new Thickness(14, 13, 14, 16);

        _controls.Margin = new Thickness(0);

        // Compact narrows the strip past the point where a device name is
        // readable, so the button drops to its icon, with the full name on
        // its tooltip.
        _device.Compact = mode == LayoutMode.Compact;

        _appsHeader.HorizontalAlignment = HorizontalAlignment.Stretch;
        _appsHeader.VerticalAlignment = VerticalAlignment.Top;
        _appsHeader.Margin = new Thickness(0, 12, 0, 0);
        _chips.Margin = new Thickness(0, 6, 0, 0);

        Tree.Detach(_fader);
        _faderHost.Children.Clear();

        Place(_heading, 0, 0);
        Place(_fader, 1, 0);
        Place(_controls, 2, 0);
        Place(_appsHeader, 3, 0);
        Place(_chips, 4, 0);
    }

    /// <summary>
    /// Portrait: a full-width row, the fader on a line of its own.
    /// </summary>
    /// <remarks>
    /// <para>
    /// <b>The fader is the same control, rotated.</b> A <c>LayoutTransform</c>
    /// of 90 degrees is not a drawing trick here: WPF measures the transformed
    /// box, so the fader's local <i>height</i> becomes the row's width, and it
    /// delivers mouse input in the element's own untransformed coordinates, so
    /// every drag, detent and dwell works with no code aware of the rotation.
    /// </para>
    /// <para>
    /// <b>Why the fader gets a line of its own.</b> The obvious row puts the
    /// name at the left, the fader across the centre and the mute and device
    /// buttons at the right. At 456 logical pixels - the width this layout
    /// exists for - the name takes 104, the device button will not go below
    /// 150, and <b>the fader is left with 40 pixels</b>. Five detents and a grip
    /// do not fit in 40 pixels; nor does a usable drag.
    ///
    /// So the name and the two buttons share the first line, and the fader
    /// takes the whole width of the second: the control that matters most gets
    /// the space.
    /// </para>
    /// <para>
    /// The direction is why it is +90 and not -90. Rotating clockwise maps the
    /// fader's local bottom - which is zero - to the left of the screen, so the
    /// horizontal fader reads low-to-high left-to-right like every other
    /// horizontal control. Rotating the other way would have put zero on the
    /// right.
    /// </para>
    /// </remarks>
    private void ArrangeAsRow()
    {
        _layout.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });   // name, apps, controls
        _layout.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });   // the fader, full width
        _layout.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });   // chips, once opened

        _layout.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });// name
        _layout.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });                     // apps
        _layout.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });                     // controls

        // Name and readout side by side rather than stacked. Two lines of text
        // cost the row more height than the mute button beside them needs, and
        // that height is paid six times over.
        _heading.Orientation = Orientation.Horizontal;
        _heading.HorizontalAlignment = HorizontalAlignment.Left;
        _heading.VerticalAlignment = VerticalAlignment.Center;
        _name.HorizontalAlignment = HorizontalAlignment.Left;
        _name.VerticalAlignment = VerticalAlignment.Center;
        _readout.HorizontalAlignment = HorizontalAlignment.Left;
        _readout.VerticalAlignment = VerticalAlignment.Center;
        _readout.Margin = new Thickness(9, 1, 0, 0);

        _fader.LayoutTransform = new RotateTransform(90);
        _fader.VerticalAlignment = VerticalAlignment.Top;
        _fader.HorizontalAlignment = HorizontalAlignment.Left;

        // The negative bottom margin reclaims a band the fader never draws in.
        //
        // A vertical fader is deliberately lopsided: the detent paddles run
        // down the LEFT of the track and nothing but the cap's own half-width
        // sits to the right of it. Standing the control up, that unused strip
        // becomes thirty-odd pixels of empty row under every channel - and six
        // of those is a screenful.
        //
        // Cropping it here rather than narrowing the fader is deliberate: the
        // paddle geometry is measured from the control's centre, so a narrower
        // fader would move the markers rather than remove the gap.
        _fader.Margin = new Thickness(0, 0, 0, -(UnusedBand - TrackBreathing));

        // Tighter than a column strip. A row has no vertical space to spare and
        // nothing below the fader to separate it from.
        _accentRule.Height = 22;
        _layout.Margin = new Thickness(14, 9, 14, 9);

        _controls.Margin = new Thickness(0);
        _controls.VerticalAlignment = VerticalAlignment.Center;

        // There is room across a full-width row, so the device name comes back.
        _device.Compact = false;
        _device.MinWidth = 150;

        // The apps header shares the first line rather than taking a third
        // one, and this is the single biggest saving in the stacked layout.
        //
        // It is waste removed rather than a compromise. "3 apps" alone on a
        // line costs about 34 pixels, six times over - a fifth of a portrait
        // window spent on six short labels - and on the first line it costs
        // nothing at all, because the mute button beside it is taller.
        _appsHeader.HorizontalAlignment = HorizontalAlignment.Right;
        _appsHeader.VerticalAlignment = VerticalAlignment.Center;
        _appsHeader.Margin = new Thickness(10, 0, 10, 0);
        _chips.Margin = new Thickness(0, 8, 0, 0);

        Tree.Detach(_fader);
        _faderHost.Children.Clear();
        _faderHost.Children.Add(_fader);
        _faderHost.Margin = new Thickness(0, 5, 0, 0);
        _faderHost.VerticalAlignment = VerticalAlignment.Top;

        if (_minimal)
        {
            ArrangeAsOneLine();
            return;
        }

        Place(_heading, 0, 0);
        Place(_controls, 0, 2);

        Place(_faderHost, 1, 0);
        Grid.SetColumnSpan(_faderHost, 3);

        Place(_appsHeader, 0, 1);
        Place(_chips, 2, 0);
        Grid.SetColumnSpan(_chips, 3);
    }

    /// <summary>
    /// The quick mixer's row: name, fader and mute on a single line.
    /// </summary>
    /// <remarks>
    /// The stacked row gives the fader a line of its own because the name, the
    /// apps count and two buttons leave it too little room on the first one.
    /// With no apps and no device button there is room, and a line per channel
    /// is what makes the quick mixer small: its first version stood 881 pixels
    /// tall, which is not a flyout.
    /// </remarks>
    private void ArrangeAsOneLine()
    {
        _layout.RowDefinitions.Clear();
        _layout.ColumnDefinitions.Clear();
        _layout.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        _layout.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(74) });                  // name
        _layout.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) }); // fader
        _layout.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });                     // mute

        // Name above its reading, since the two no longer have the row to
        // themselves.
        _heading.Orientation = Orientation.Vertical;
        _heading.VerticalAlignment = VerticalAlignment.Center;
        _readout.Margin = new Thickness(0, 2, 0, 0);

        _faderHost.Margin = new Thickness(6, 0, 10, 0);
        _faderHost.VerticalAlignment = VerticalAlignment.Center;

        // Only the mute is left in the control pair, so the gap that separated
        // it from the device button would be dead space.
        _controls.ColumnDefinitions[1].Width = new GridLength(0);

        Place(_heading, 0, 0);
        Place(_faderHost, 0, 1);
        Place(_controls, 0, 2);
    }

    /// <summary>
    /// How much of the fader's width is on the far side of the track, unused.
    /// </summary>
    /// <remarks>
    /// Derived rather than guessed: the paddles reach to within
    /// <see cref="Detents.EdgeInset"/> of one edge, and the only thing on the
    /// other side of the track is half the cap. Everything past that is empty.
    /// </remarks>
    private const double UnusedBand =
        FaderThickness / 2 - Detents.EdgeInset - 13;

    /// <summary>How much of the unused band to leave, so the track is not flush.</summary>
    private const double TrackBreathing = 8;

    private void Place(UIElement child, int row, int column)
    {
        Grid.SetRow(child, row);
        Grid.SetColumn(child, column);
        Grid.SetColumnSpan(child, 1);
        _layout.Children.Add(child);
    }

    private void ClearDropHighlight()
    {
        Background = (Brush)Application.Current.Resources["Card"];
        BorderBrush = (Brush)Application.Current.Resources["Line"];
    }

    /// <summary>
    /// Paint the accent: a lit top edge fading down into the card.
    /// </summary>
    /// <remarks>
    /// <para>
    /// <b>The transparent stop keeps the accent's own red, green and blue.</b>
    /// Fading to <c>Colors.Transparent</c> looks like the obvious thing to do
    /// and is wrong: it is <c>#00FFFFFF</c>, so the gradient interpolates
    /// toward white and the band washes out pale before it disappears. Fading
    /// to the same colour at zero alpha fades only the alpha, which is what
    /// "fading in to the main body of the box" actually means.
    /// </para>
    /// <para>
    /// The first two stops hold full strength for a couple of pixels. Without
    /// them the gradient starts falling immediately and the card loses its top
    /// edge — the colour reads as a smear rather than as a lit rule with a
    /// glow under it. The steep drop that follows is what keeps the wash from
    /// tinting the whole header.
    /// </para>
    /// </remarks>
    private void PaintRule(Brush accent, bool muted)
    {
        if (ReferenceEquals(_rulePainted.Accent, accent) && _rulePainted.Muted == muted)
        {
            return;
        }

        _rulePainted = (accent, muted);

        var colour = accent is SolidColorBrush solid ? solid.Color : Colors.Gray;

        var brush = new LinearGradientBrush
        {
            StartPoint = new Point(0.5, 0),
            EndPoint = new Point(0.5, 1),
        };
        brush.GradientStops.Add(new GradientStop(colour, 0.0));
        brush.GradientStops.Add(new GradientStop(colour, 3.0 / HeaderFade));
        brush.GradientStops.Add(new GradientStop(Alpha(colour, 0x3A), 0.17));
        brush.GradientStops.Add(new GradientStop(Alpha(colour, 0x00), 1.0));
        brush.Opacity = muted ? 0.45 : 1.0;
        brush.Freeze();

        _accentRule.Fill = brush;
    }

    private static Color Alpha(Color colour, byte alpha) =>
        Color.FromArgb(alpha, colour.R, colour.G, colour.B);

    /// <summary>
    /// Show these applications, and this many of them.
    /// </summary>
    /// <remarks>
    /// <paramref name="mine"/> is null when the update did not carry session
    /// information at all, in which case what is already on screen is still the
    /// best answer and nothing is touched.
    /// </remarks>
    public void SetApps(IReadOnlyList<SessionState>? mine)
    {
        if (mine is null)
        {
            return;
        }

        _appsCount.Text = Describe(mine.Count);

        // With nothing to show, the row is a label rather than a control: no
        // chevron, no hand cursor, no hover. A disclosure triangle that opens
        // onto nothing is the same broken promise the old static "2 apps" text
        // made, in the other direction.
        var any = mine.Count > 0;
        _appsChevron.Visibility = any ? Visibility.Visible : Visibility.Collapsed;
        _appsHeader.IsHitTestVisible = any;
        _appsHeader.Cursor = any ? Cursors.Hand : Cursors.Arrow;

        var accent = AccentFor(ChannelId);
        var order = mine.Select(s => s.Executable).ToList();

        if (order.SequenceEqual(_chipOrder))
        {
            // Same applications: refresh in place. Only the playing flag can
            // have changed, and rebuilding a chip somebody might be dragging
            // would drop the drag.
            foreach (var child in _chips.Children)
            {
                if (child is AppChip chip)
                {
                    var session = mine.FirstOrDefault(s => s.Executable == chip.Executable);
                    if (session is not null)
                    {
                        chip.Update(session, accent);
                    }
                }
            }
            return;
        }

        _chipOrder = order;
        _chips.Children.Clear();
        foreach (var session in mine)
        {
            var chip = new AppChip();
            chip.Update(session, accent);
            _chips.Children.Add(chip);
        }

        ShowChips();
    }

    /// <summary>
    /// The chips take room only when the list is open <i>and</i> has something
    /// in it.
    /// </summary>
    /// <remarks>
    /// An open list whose apps had all closed still took its top margin, so
    /// that one strip's fader stood six pixels shorter than its neighbours' and
    /// its buttons six pixels higher. Invisible with one strip on its own; plain
    /// with seven side by side.
    /// </remarks>
    private void ShowChips() =>
        _chips.Visibility = !_collapsed && _chips.Children.Count > 0
            ? Visibility.Visible
            : Visibility.Collapsed;

    /// <summary>Open or close the apps list.</summary>
    public void SetCollapsed(bool collapsed)
    {
        _collapsed = collapsed;
        ShowChips();
        _appsChevron.LayoutTransform = collapsed
            ? Transform.Identity
            : new RotateTransform(180);
    }

    private static string Describe(int count) => count switch
    {
        0 => "No apps",
        1 => "1 app",
        _ => $"{count} apps",
    };

    /// <summary>Apply new state. Never rebuilds anything.</summary>
    /// <param name="ceiling">
    /// Master's level, which limits every playback channel. 1.0 for Master
    /// itself and for input channels, which it does not apply to.
    /// </param>
    public void Update(
        ChannelState channel,
        IReadOnlyList<DeviceState> devices,
        IReadOnlyList<SessionState>? mine,
        double ceiling)
    {
        ChannelId = channel.Id;
        IsInput = channel.IsInput;

        var accent = AccentFor(channel.Id);
        PaintRule(channel.Muted ? (Brush)Application.Current.Resources["MutedGrey"] : accent, channel.Muted);

        _name.Text = channel.Name;
        _name.Foreground = (Brush)Application.Current.Resources[channel.Muted ? "TextFaint" : "Text"];

        _fader.Accent = accent;
        _fader.Muted = channel.Muted;
        _fader.Ceiling = ceiling;
        // The EFFECTIVE level, not the set one: a channel held down by Master
        // has to look held down, which is the whole point of a ceiling.
        _fader.Value = channel.EffectiveVolume;

        // Not taken from the fader's own value: while a drag is in flight the
        // fader is the authority on what to show, and it updates this itself.
        _readout.Text = _fader.DisplayText;
        _readout.Foreground = (Brush)Application.Current.Resources[channel.Muted ? "Warn" : "TextDim"];

        _mute.Background = (Brush)Application.Current.Resources[channel.Muted ? "Warn" : "Raised"];
        Icons.SetSpeaker(
            _muteIcon,
            channel.Muted,
            (Brush)Application.Current.Resources[channel.Muted ? "Ground" : "TextDim"]);

        _device.Update(channel, devices);
        _removable = channel.Removable;
        _channelName = channel.Name;

        SetCollapsed(channel.Collapsed);
        SetApps(mine);
    }

    public void SetMeter(double level) => _fader.Meter = level;

    /// <summary>
    /// The button needs the top corner to itself, which only the column
    /// layouts give it; a stacked row has its device button there.
    /// </summary>
    private void ShowMoreButton() =>
        _more.Visibility = _minimal || _mode == LayoutMode.Stacked
            ? Visibility.Collapsed
            : Visibility.Visible;

    /// <summary>Rename, the Game/Chat mix, and removing an added channel.</summary>
    private void OpenMenu(UIElement target)
    {
        var menu = new PopupMenu(target, 220);

        menu.Row("Rename", BeginRename, "Or double-click the name");

        // Master is a ceiling over the other channels, not a peer of them, and
        // the Mic is an input; the core refuses both as a side of the mix.
        if (Commands is { } commands && !IsInput && ChannelId != "master")
        {
            // Offered as a partner rather than as a side. Which end of the
            // control each channel sits at follows the strips' order, not this
            // choice, so "left" and "right" here would be promises the window
            // could not keep.
            var (a, b) = commands.MixPair;
            var partner = ChannelId == a ? b : ChannelId == b ? a : null;
            var others = commands.MixCandidates.Where(c => c.Id != ChannelId).ToList();

            if (others.Count > 0)
            {
                menu.Separator();
                menu.Heading("Balance in the mix against");
                foreach (var (id, name) in others)
                {
                    menu.Row(
                        name,
                        () => commands.SetMixPair(ChannelId, id),
                        $"The mix control will balance {_channelName} against {name}, starting from the centre",
                        id == partner ? "current" : null);
                }
            }
        }

        if (_removable && Commands is { } remover)
        {
            menu.Separator();
            menu.ConfirmRow(
                "Remove channel",
                $"Click again to remove {_channelName}",
                () => remover.Remove(ChannelId),
                "Its apps go back to To be routed and keep playing as they are.");
        }

        menu.Open();
    }

    /// <summary>
    /// Swap the name for a text box holding it. Enter or clicking away keeps
    /// the new name; Esc keeps the old one.
    /// </summary>
    /// <remarks>
    /// The box takes the name's place in the heading rather than floating over
    /// it, so it is wherever the name is in every layout - centred over a
    /// column, or at the left of a row - without knowing which.
    /// </remarks>
    private void BeginRename()
    {
        if (_editor is not null)
        {
            return;
        }

        var editor = new TextBox
        {
            Text = _name.Text,
            FontSize = _name.FontSize,
            FontWeight = _name.FontWeight,
            FontFamily = _name.FontFamily,
            MaxLength = 24,
            MinWidth = 70,
            Padding = new Thickness(4, 0, 4, 0),
            HorizontalAlignment = _name.HorizontalAlignment,
            HorizontalContentAlignment = HorizontalAlignment.Center,
            Background = (Brush)Application.Current.Resources["Raised"],
            Foreground = (Brush)Application.Current.Resources["Text"],
            CaretBrush = (Brush)Application.Current.Resources["Text"],
            BorderBrush = (Brush)Application.Current.Resources["LineStrong"],
            BorderThickness = new Thickness(1),
        };

        editor.KeyDown += (_, e) =>
        {
            if (e.Key == Key.Enter)
            {
                e.Handled = true;
                EndRename(keep: true);
            }
            else if (e.Key == Key.Escape)
            {
                e.Handled = true;
                EndRename(keep: false);
            }
        };
        editor.LostKeyboardFocus += (_, _) => EndRename(keep: true);

        var index = _heading.Children.IndexOf(_name);
        _heading.Children.RemoveAt(index);
        _heading.Children.Insert(index, editor);
        _editor = editor;

        Dispatcher.BeginInvoke(() =>
        {
            editor.Focus();
            editor.SelectAll();
        }, System.Windows.Threading.DispatcherPriority.Input);
    }

    private void EndRename(bool keep)
    {
        if (_editor is not { } editor)
        {
            return;
        }
        _editor = null;

        var index = _heading.Children.IndexOf(editor);
        if (index >= 0)
        {
            _heading.Children.RemoveAt(index);
            _heading.Children.Insert(index, _name);
        }

        var name = editor.Text.Trim();
        if (keep && name.Length > 0 && name != _name.Text)
        {
            // Shown at once; the core's echo confirms it, or puts the old name
            // back if it refused.
            _name.Text = name;
            RenameRequested?.Invoke(ChannelId, name);
        }
    }

    /// <summary>
    /// The channel's accent, or a neutral one for a channel the user added.
    /// </summary>
    private static Brush AccentFor(string channelId)
    {
        var key = "Accent." + channelId;
        return Application.Current.Resources.Contains(key)
            ? (Brush)Application.Current.Resources[key]
            : (Brush)Application.Current.Resources["AccentFallback"];
    }
}
