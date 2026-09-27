using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Input;
using System.Windows.Media;

using Lanes.Ui.Api;

namespace Lanes.Ui.Controls;

/// <summary>
/// The output (or input) picker on a channel strip.
/// </summary>
/// <remarks>
/// <para>
/// A button that opens a list, rather than a <c>ComboBox</c>, for two reasons.
/// It has to shrink - a stock combo box carries a large minimum width, and six
/// of them would set the whole window's minimum, which the narrow layouts
/// cannot afford. And it has to carry a warning state, for when the preferred device is absent
/// and the channel is running on a fallback.
/// </para>
/// <para>
/// <b>"System default" is deliberately not offered.</b> A channel points at a
/// device, not at a moving target that changes underneath it whenever Windows
/// switches its default.
/// </para>
/// </remarks>
public sealed class DeviceButton : Border
{
    private readonly TextBlock _label = new()
    {
        VerticalAlignment = VerticalAlignment.Center,
        TextTrimming = TextTrimming.CharacterEllipsis,
    };

    /// <summary>Stands in for the name when there is no room for one.</summary>
    private readonly Grid _icon =
        Icons.Speaker(false, (Brush)Application.Current.Resources["TextDim"], 15);

    private readonly Popup _popup;
    private readonly StackPanel _items = new();
    private IReadOnlyList<DeviceState> _devices = [];

    /// <summary>A device was chosen. The argument is its id.</summary>
    public event Action<string>? DeviceChosen;

    private bool _compact;

    /// <summary>
    /// Show a speaker icon instead of the device's name.
    /// </summary>
    /// <remarks>
    /// For the compact layout, where a strip is too narrow for a device name to
    /// be anything but a truncation. The full name stays on the tooltip, so
    /// nothing is lost that was not already unreadable.
    ///
    /// The chevron stays. Without it this is a picture of a speaker, and the
    /// one thing the control has to say is that it opens something.
    /// </remarks>
    public bool Compact
    {
        get => _compact;
        set
        {
            if (_compact == value)
            {
                return;
            }

            _compact = value;
            _label.Visibility = value ? Visibility.Collapsed : Visibility.Visible;
            _icon.Visibility = value ? Visibility.Visible : Visibility.Collapsed;
            MinWidth = value ? 44 : 34;
        }
    }

    public DeviceButton()
    {
        Background = (Brush)Application.Current.Resources["Raised"];
        CornerRadius = new CornerRadius(9);
        MinWidth = 34;
        Height = 34;
        Cursor = Cursors.Hand;
        SnapsToDevicePixels = true;

        var layout = new DockPanel { LastChildFill = true, Margin = new Thickness(11, 0, 10, 0) };

        var chevron = Icons.Chevron(Icons.Direction.Down, (Brush)Application.Current.Resources["TextGhost"], 9);
        chevron.VerticalAlignment = VerticalAlignment.Center;
        chevron.Margin = new Thickness(7, 0, 0, 0);
        DockPanel.SetDock(chevron, Dock.Right);

        DockPanel.SetDock(_icon, Dock.Left);
        _icon.VerticalAlignment = VerticalAlignment.Center;
        _icon.Visibility = Visibility.Collapsed;

        layout.Children.Add(chevron);
        layout.Children.Add(_icon);
        layout.Children.Add(_label);
        Child = layout;

        // Every other control in this window answers the pointer. One that does
        // not reads as a caption with a name in it.
        MouseEnter += (_, _) => Opacity = 0.8;
        MouseLeave += (_, _) => Opacity = 1.0;

        _popup = new Popup
        {
            PlacementTarget = this,
            Placement = PlacementMode.Top,
            StaysOpen = false,
            AllowsTransparency = true,
            PopupAnimation = PopupAnimation.Fade,
            // Follows the window when it moves, rather than hanging in space.
            HorizontalOffset = 0,
            VerticalOffset = -6,
        };

        var shell = new Border
        {
            Background = (Brush)Application.Current.Resources["PopupGround"],
            BorderBrush = (Brush)Application.Current.Resources["LineStrong"],
            BorderThickness = new Thickness(1),
            CornerRadius = new CornerRadius(10),
            Padding = new Thickness(5),
            MinWidth = 190,
            Effect = new System.Windows.Media.Effects.DropShadowEffect
            {
                BlurRadius = 18,
                ShadowDepth = 2,
                Opacity = (double)Application.Current.Resources["ShadowOpacity"],
                Color = Colors.Black,
            },
            Child = _items,
        };

        _popup.Child = shell;

        MouseLeftButtonUp += (_, _) =>
        {
            if (_devices.Count > 0)
            {
                _popup.IsOpen = true;
            }
        };
    }

    /// <summary>What the list was last built from.</summary>
    /// <remarks>
    /// State arrives about thirty times a second while meters run, and the list
    /// is only worth rebuilding when something it shows has changed: which
    /// devices exist, whether each is plugged in, and which is chosen.
    /// Comparing ids alone would miss a device going from present to
    /// unplugged.
    /// </remarks>
    private string _signature = "";

    /// <summary>Refresh, rebuilding the list only when what it shows changed.</summary>
    public void Update(ChannelState channel, IReadOnlyList<DeviceState> devices)
    {
        // An input channel must be offered inputs. Handing every strip the
        // output list is how the last window ended up offering speakers as a
        // microphone.
        var wanted = channel.IsInput ? "input" : "output";
        var filtered = devices.Where(d => d.Direction == wanted).ToList();

        var chosen = channel.EffectiveDevice ?? channel.TargetDevice;
        _label.Text = chosen is null ? (channel.IsInput ? "Default input" : "Choose device") : Short(chosen.Name);
        _label.Foreground = (Brush)Application.Current.Resources[
            channel.OnFallback ? "Warn" : chosen is null ? "TextGhost" : "TextDim"];

        Background = (Brush)Application.Current.Resources[channel.OnFallback ? "WarnGround" : "Raised"];

        // On a fallback, the tooltip says why. An amber button that only names
        // the device it is using leaves the user to work out that something
        // else is missing - and that is the one thing they need to know.
        ToolTip = channel.OnFallback && channel.TargetDevice is { } missing
            ? $"{chosen?.Name ?? "System default"}\n{missing.Name} is unplugged - using this instead"
            : chosen?.Name;

        // The row in bold is what the user chose, not what is playing. On a
        // fallback those differ, and the choice is the one the list is for.
        var selected = channel.TargetDevice?.Id ?? chosen?.Id;

        _devices = filtered;

        var signature = string.Join(";", filtered.Select(d => $"{d.Id}:{d.Present}"))
            + "|" + selected;

        if (signature == _signature)
        {
            return;
        }

        _signature = signature;
        _items.Children.Clear();

        foreach (var device in filtered)
        {
            _items.Children.Add(BuildRow(device, selected));
        }
    }

    private Border BuildRow(DeviceState device, string? currentId)
    {
        var text = new TextBlock
        {
            Text = Short(device.Name),
            VerticalAlignment = VerticalAlignment.Center,
            TextTrimming = TextTrimming.CharacterEllipsis,
            FontWeight = device.Id == currentId ? FontWeights.Bold : FontWeights.Normal,
        };

        var content = new DockPanel { Margin = new Thickness(10, 0, 10, 0) };

        // Unplugged devices stay in the list, dimmed, and stay choosable:
        // pointing a channel at a DAC that is switched off is the reason they
        // are offered at all. Saying so in words rather than colour alone,
        // because "grey" could mean disabled, and it does not.
        if (!device.Present)
        {
            var note = new TextBlock
            {
                Text = "unplugged",
                Margin = new Thickness(8, 0, 0, 0),
                VerticalAlignment = VerticalAlignment.Center,
                FontSize = (double)Application.Current.Resources["SizeTiny"],
                Foreground = (Brush)Application.Current.Resources["TextGhost"],
            };
            DockPanel.SetDock(note, Dock.Right);
            content.Children.Add(note);
            text.Foreground = (Brush)Application.Current.Resources["TextFaint"];
        }

        content.Children.Add(text);

        var row = Row(content, device.Name);
        row.MouseLeftButtonUp += (_, _) =>
        {
            _popup.IsOpen = false;
            DeviceChosen?.Invoke(device.Id);
        };
        return row;
    }

    /// <summary>The hover and hit behaviour every row in the list shares.</summary>
    private static Border Row(UIElement content, string tooltip)
    {
        var row = new Border
        {
            Height = 30,
            CornerRadius = new CornerRadius(7),
            Background = Brushes.Transparent,
            Cursor = Cursors.Hand,
            Child = content,
            ToolTip = tooltip,
        };

        var hover = (Brush)Application.Current.Resources["LineStrong"];
        row.MouseEnter += (_, _) => row.Background = hover;
        row.MouseLeave += (_, _) => row.Background = Brushes.Transparent;
        return row;
    }

    /// <summary>
    /// "Speakers (Realtek(R) Audio)" reads better as "Realtek(R) Audio".
    /// </summary>
    /// <remarks>
    /// Windows wraps the useful part of an endpoint name in the generic part.
    /// The full name is still on the tooltip and in the list, so nothing is
    /// lost by showing the half that identifies the hardware.
    /// </remarks>
    private static string Short(string name)
    {
        var open = name.IndexOf('(');
        var close = name.LastIndexOf(')');
        return close > open + 1 && open >= 0 ? name[(open + 1)..close] : name;
    }
}
