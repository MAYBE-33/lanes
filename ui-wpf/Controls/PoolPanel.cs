using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Input;
using System.Windows.Media;

using Lanes.Ui.Api;

namespace Lanes.Ui.Controls;

/// <summary>
/// "To be routed": every application playing outside any channel.
/// </summary>
/// <remarks>
/// <para>
/// <b>Collapsed to a rail by default, and it never opens itself.</b> That is a
/// standing rule for this window — the user's words were "everything should be
/// hidden or condensed unless manually opened". A pool that expanded whenever
/// something new arrived would be delightful once and irritating thereafter.
/// </para>
/// <para>
/// But an unrouted application is playing at full volume outside the user's
/// control, so it cannot hide silently either. The collapsed rail carries a
/// count that turns <b>amber</b> whenever anything is waiting. The pool may get
/// out of the way; it may not go quiet.
/// </para>
/// </remarks>
public sealed class PoolPanel : Border
{
    private readonly Border _rail = new();
    /// <summary>
    /// The open pool. A dock panel so the chip list takes whatever height is
    /// left and scrolls within it: since the device settings arrived the pool
    /// has half the column, and a stack panel would simply have run past its
    /// half and over the panel beneath.
    /// </summary>
    private readonly DockPanel _expanded = new() { LastChildFill = true };
    private readonly StackPanel _chips = new();
    private readonly TextBlock _railCount = new();
    private readonly Border _railCountShell = new();
    private readonly TextBlock _expandedCount = new();
    private readonly TextBlock _railLabel = new();

    private bool _open;
    private IReadOnlyList<string> _unrouted = [];

    /// <summary>The narrow form: a button in the header band.</summary>
    private readonly Border _button = new();
    private readonly TextBlock _buttonText = new();
    private readonly Border _buttonCountShell = new();
    private readonly TextBlock _buttonCount = new();

    /// <summary>
    /// The chips, for the narrow form, shown over the window rather than beside
    /// it.
    /// </summary>
    /// <remarks>
    /// A popup and not another column, because in the layouts that use this
    /// there is no spare column: that is what made them narrow.
    /// </remarks>
    private readonly Popup _overlay = new();

    private LayoutMode _mode = LayoutMode.Full;

    /// <summary>An application was dropped here, meaning "stop managing it".</summary>
    public event Action<IReadOnlyList<string>>? UnassignRequested;

    public PoolPanel()
    {
        // The panel itself stays transparent and the two states carry their own
        // chrome: the rail is a card, the expanded pool is a plain column that
        // sits beside the strips.
        Background = Brushes.Transparent;
        AllowDrop = true;

        BuildRail();
        BuildExpanded();
        BuildButton();
        Child = _rail;

        DragOver += (_, e) =>
        {
            e.Effects = e.Data.GetDataPresent(DataFormats.StringFormat)
                ? DragDropEffects.Move
                : DragDropEffects.None;
            e.Handled = true;
        };

        Drop += (_, e) =>
        {
            if (AppChip.Dragged(e.Data) is { Count: > 0 } executables)
            {
                UnassignRequested?.Invoke(executables);
            }
            e.Handled = true;
        };
    }

    private void BuildRail()
    {
        // A column, laid out like a channel strip rather than a stack of
        // leftovers pushed against the left edge. It reads as the first column
        // of the mixer, which is what it is.
        var stack = new StackPanel
        {
            Width = 46,
            HorizontalAlignment = HorizontalAlignment.Center,
            Margin = new Thickness(0, 14, 0, 14),
        };

        var chevron = Icons.Chevron(Icons.Direction.Right, (Brush)Application.Current.Resources["TextFaint"]);
        chevron.HorizontalAlignment = HorizontalAlignment.Center;
        chevron.VerticalAlignment = VerticalAlignment.Center;

        var toggle = new Border
        {
            Width = 30,
            Height = 30,
            CornerRadius = new CornerRadius(10),
            Background = (Brush)Application.Current.Resources["Card"],
            BorderBrush = (Brush)Application.Current.Resources["Line"],
            BorderThickness = new Thickness(1),
            Cursor = Cursors.Hand,
            HorizontalAlignment = HorizontalAlignment.Center,
            ToolTip = "Show applications with no channel",
            Child = chevron,
        };
        toggle.MouseLeftButtonUp += (_, _) => Toggle();
        toggle.MouseEnter += (_, _) => toggle.Background = (Brush)Application.Current.Resources["Hover"];
        toggle.MouseLeave += (_, _) => toggle.Background = (Brush)Application.Current.Resources["Card"];
        stack.Children.Add(toggle);

        _railCount.HorizontalAlignment = HorizontalAlignment.Center;
        _railCount.VerticalAlignment = VerticalAlignment.Center;
        _railCount.FontWeight = FontWeights.Bold;
        _railCount.FontSize = (double)Application.Current.Resources["SizeTiny"];

        _railCountShell.Width = 30;
        _railCountShell.Height = 21;
        _railCountShell.CornerRadius = new CornerRadius(8);
        _railCountShell.Margin = new Thickness(0, 10, 0, 0);
        _railCountShell.HorizontalAlignment = HorizontalAlignment.Center;
        _railCountShell.Child = _railCount;
        stack.Children.Add(_railCountShell);

        // Rotated with a transform, which does not change the layout box - so
        // the text is given explicit dimensions the other way round and centred
        // inside the rail. Get this wrong and the label spills out of the rail
        // over the first channel strip.
        _railLabel.Text = "To be routed";
        _railLabel.Foreground = (Brush)Application.Current.Resources["TextGhost"];
        _railLabel.FontSize = (double)Application.Current.Resources["SizeTiny"];
        _railLabel.LayoutTransform = new RotateTransform(90);
        _railLabel.Margin = new Thickness(0, 18, 0, 0);
        _railLabel.HorizontalAlignment = HorizontalAlignment.Center;
        stack.Children.Add(_railLabel);

        // The same card the channel strips are, so the collapsed pool is a
        // column of the mixer rather than three controls floating on the
        // background.
        _rail.Background = (Brush)Application.Current.Resources["Card"];
        _rail.BorderBrush = (Brush)Application.Current.Resources["Line"];
        _rail.BorderThickness = new Thickness(1);
        _rail.CornerRadius = new CornerRadius(14);
        _rail.Child = stack;
    }

    private void BuildExpanded()
    {
        _expanded.Width = 194;

        var header = new DockPanel { LastChildFill = true, Height = 22 };
        var toggle = new Border
        {
            Cursor = Cursors.Hand,
            Background = Brushes.Transparent,
            VerticalAlignment = VerticalAlignment.Center,
            Padding = new Thickness(2, 4, 9, 4),
            Child = Icons.Chevron(Icons.Direction.Left, (Brush)Application.Current.Resources["TextFaint"]),
        };
        toggle.MouseLeftButtonUp += (_, _) => Toggle();
        DockPanel.SetDock(toggle, Dock.Left);

        var title = new TextBlock
        {
            Text = "To be routed",
            FontSize = (double)Application.Current.Resources["SizeBody"],
            FontWeight = FontWeights.SemiBold,
            VerticalAlignment = VerticalAlignment.Center,
        };
        DockPanel.SetDock(title, Dock.Left);

        _expandedCount.Foreground = (Brush)Application.Current.Resources["Warn"];
        _expandedCount.FontWeight = FontWeights.SemiBold;
        _expandedCount.VerticalAlignment = VerticalAlignment.Center;
        _expandedCount.HorizontalAlignment = HorizontalAlignment.Right;

        header.Children.Add(toggle);
        header.Children.Add(title);
        header.Children.Add(_expandedCount);
        DockPanel.SetDock(header, Dock.Top);
        _expanded.Children.Add(header);

        var description = new TextBlock
        {
            Text = "Playing at their own volume, outside any channel.",
            Foreground = (Brush)Application.Current.Resources["TextDim"],
            TextWrapping = TextWrapping.Wrap,
            Margin = new Thickness(0, 12, 0, 12),
        };
        DockPanel.SetDock(description, Dock.Top);
        _expanded.Children.Add(description);

        var hint = new TextBlock
        {
            Text = "Drag onto a channel to create a rule.",
            Foreground = (Brush)Application.Current.Resources["TextGhost"],
            FontSize = (double)Application.Current.Resources["SizeTiny"],
            TextWrapping = TextWrapping.Wrap,
            Margin = new Thickness(0, 12, 0, 0),
        };
        DockPanel.SetDock(hint, Dock.Bottom);
        _expanded.Children.Add(hint);

        // Last, so it fills what is left.
        _expanded.Children.Add(new ScrollViewer
        {
            VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
            MaxHeight = 420,
            Content = _chips,
        });
    }

    private void Toggle()
    {
        _open = !_open;

        if (_mode == LayoutMode.Full)
        {
            Child = _open ? _expanded : _rail;
            return;
        }

        // Narrow: the button stays put and the chips appear over the window.
        _overlay.IsOpen = _open;
    }

    /// <summary>
    /// Switch between the rail beside the strips and the button in the band.
    /// </summary>
    /// <remarks>
    /// The expanded chip list is the same <see cref="StackPanel"/> in both, so
    /// a drag in progress survives a reflow and the two can never disagree
    /// about what is in the pool. Only its container changes.
    /// </remarks>
    public void SetMode(LayoutMode mode)
    {
        if (mode == _mode)
        {
            return;
        }

        _mode = mode;
        _open = false;
        _overlay.IsOpen = false;

        // A child has one parent, so the expanded column is detached from
        // whichever holder had it before being handed to the other.
        if (_expanded.Parent is Decorator previous)
        {
            previous.Child = null;
        }

        if (mode == LayoutMode.Full)
        {
            _overlay.Child = null;
            Child = _rail;
        }
        else
        {
            _overlay.Child = new Border
            {
                Background = (Brush)Application.Current.Resources["PopupGround"],
                BorderBrush = (Brush)Application.Current.Resources["LineStrong"],
                BorderThickness = new Thickness(1),
                CornerRadius = new CornerRadius(12),
                Padding = new Thickness(14),
                Child = _expanded,
                Effect = new System.Windows.Media.Effects.DropShadowEffect
                {
                    BlurRadius = 20,
                    ShadowDepth = 3,
                    Opacity = (double)Application.Current.Resources["ShadowOpacity"],
                    Color = Colors.Black,
                },
            };

            Child = _button;
        }
    }

    /// <summary>
    /// The narrow form: one button, with the same amber count the rail carries.
    /// </summary>
    /// <remarks>
    /// The count is the part that cannot be dropped. An unrouted application is
    /// playing at full volume outside the user's control, and the whole reason
    /// the pool is allowed to fold away at all is that it keeps saying so.
    /// </remarks>
    private void BuildButton()
    {
        _buttonText.Text = "To be routed";
        _buttonText.VerticalAlignment = VerticalAlignment.Center;
        _buttonText.FontSize = (double)Application.Current.Resources["SizeSmall"];
        _buttonText.Foreground = (Brush)Application.Current.Resources["TextDim"];

        _buttonCount.HorizontalAlignment = HorizontalAlignment.Center;
        _buttonCount.VerticalAlignment = VerticalAlignment.Center;
        _buttonCount.FontWeight = FontWeights.Bold;
        _buttonCount.FontSize = (double)Application.Current.Resources["SizeTiny"];

        _buttonCountShell.Width = 26;
        _buttonCountShell.Height = 19;
        _buttonCountShell.CornerRadius = new CornerRadius(7);
        _buttonCountShell.Margin = new Thickness(9, 0, 0, 0);
        _buttonCountShell.VerticalAlignment = VerticalAlignment.Center;
        _buttonCountShell.Child = _buttonCount;

        var row = new StackPanel { Orientation = Orientation.Horizontal };
        row.Children.Add(_buttonText);
        row.Children.Add(_buttonCountShell);

        _button.Background = (Brush)Application.Current.Resources["Card"];
        _button.BorderBrush = (Brush)Application.Current.Resources["Line"];
        _button.BorderThickness = new Thickness(1);
        _button.CornerRadius = new CornerRadius(14);
        _button.Padding = new Thickness(14, 10, 14, 10);
        _button.Margin = new Thickness(0, 0, StripGap, 14);
        _button.VerticalAlignment = VerticalAlignment.Bottom;
        _button.Cursor = Cursors.Hand;
        _button.Child = row;
        _button.MouseLeftButtonUp += (_, _) => Toggle();
        _button.MouseEnter += (_, _) =>
            _button.Background = (Brush)Application.Current.Resources["Hover"];
        _button.MouseLeave += (_, _) =>
            _button.Background = (Brush)Application.Current.Resources["Card"];

        _overlay.PlacementTarget = _button;
        _overlay.Placement = PlacementMode.Bottom;
        _overlay.StaysOpen = false;
        _overlay.AllowsTransparency = true;
        _overlay.PopupAnimation = PopupAnimation.Fade;
        _overlay.VerticalOffset = 6;
        _overlay.Closed += (_, _) => _open = false;
    }

    /// <summary>Half the space between two strips, matched to the window's.</summary>
    private const double StripGap = 7;

    public void Update(IReadOnlyList<SessionState> sessions)
    {
        var unrouted = sessions.Where(s => s.Channel is null).ToList();
        var names = unrouted.Select(s => s.Executable).ToList();

        var waiting = names.Count > 0;
        _railCountShell.Background = (Brush)Application.Current.Resources[waiting ? "Warn" : "Raised"];
        _railCount.Text = names.Count.ToString();
        _railCount.Foreground = (Brush)Application.Current.Resources[waiting ? "Ground" : "TextFaint"];

        _buttonCountShell.Background = (Brush)Application.Current.Resources[waiting ? "Warn" : "Raised"];
        _buttonCount.Text = names.Count.ToString();
        _buttonCount.Foreground = (Brush)Application.Current.Resources[waiting ? "Ground" : "TextFaint"];
        _railLabel.Foreground = (Brush)Application.Current.Resources[waiting ? "TextDim" : "TextGhost"];
        _expandedCount.Text = names.Count.ToString();

        // Rebuilt only when the SET changes, not when a playing flag flips —
        // rebuilding a list someone might be dragging from would drop the
        // drag.
        if (names.SequenceEqual(_unrouted))
        {
            foreach (var child in _chips.Children)
            {
                if (child is AppChip chip)
                {
                    var session = unrouted.FirstOrDefault(s => s.Executable == chip.Executable);
                    if (session is not null)
                    {
                        // No accent: an application here belongs to no channel,
                        // and there is no colour that would be honest.
                        chip.Update(session, null);
                    }
                }
            }
            return;
        }

        _unrouted = names;
        _chips.Children.Clear();
        foreach (var session in unrouted)
        {
            var chip = new AppChip();
            chip.Update(session, null);
            _chips.Children.Add(chip);
        }
    }

}
