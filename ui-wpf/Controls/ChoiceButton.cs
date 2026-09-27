using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Input;
using System.Windows.Media;

namespace Lanes.Ui.Controls;

/// <summary>
/// Pick one of a short list: a button showing the current choice, opening a list.
/// </summary>
/// <remarks>
/// The same idiom as <see cref="DeviceButton"/> and <see cref="ProfileButton"/>,
/// in a general form for the settings window. Not the stock <c>ComboBox</c>,
/// whose default template draws a light box with a light drop-down on this
/// window's near-black ground - and which the first mixer found also declares a
/// minimum width that nothing can shrink below.
/// </remarks>
public sealed class ChoiceButton : Border
{
    private readonly TextBlock _label = new()
    {
        VerticalAlignment = VerticalAlignment.Center,
        TextTrimming = TextTrimming.CharacterEllipsis,
    };

    private readonly Popup _popup;
    private readonly StackPanel _items = new();
    private IReadOnlyList<(string Value, string Label)> _choices = [];

    /// <summary>The chosen value, or null when nothing is chosen.</summary>
    public string? Value { get; private set; }

    /// <summary>Shown when nothing is chosen.</summary>
    public string Placeholder { get; set; } = "Choose…";

    /// <summary>The user chose a value.</summary>
    public event Action<string>? Chosen;

    public ChoiceButton()
    {
        Background = Res("Raised");
        CornerRadius = new CornerRadius(8);
        Height = 30;
        Cursor = Cursors.Hand;
        SnapsToDevicePixels = true;

        var chevron = Icons.Chevron(Icons.Direction.Down, Res("TextGhost"), 9);
        chevron.VerticalAlignment = VerticalAlignment.Center;
        chevron.Margin = new Thickness(7, 0, 0, 0);
        DockPanel.SetDock(chevron, Dock.Right);

        var layout = new DockPanel { LastChildFill = true, Margin = new Thickness(10, 0, 9, 0) };
        layout.Children.Add(chevron);
        layout.Children.Add(_label);
        Child = layout;

        MouseEnter += (_, _) => Opacity = 0.8;
        MouseLeave += (_, _) => Opacity = 1.0;

        _popup = new Popup
        {
            PlacementTarget = this,
            Placement = PlacementMode.Bottom,
            VerticalOffset = 4,
            StaysOpen = false,
            AllowsTransparency = true,
            PopupAnimation = PopupAnimation.Fade,
            Child = new Border
            {
                Background = Res("PopupGround"),
                BorderBrush = Res("LineStrong"),
                BorderThickness = new Thickness(1),
                CornerRadius = new CornerRadius(9),
                Padding = new Thickness(4),
                Child = new ScrollViewer
                {
                    MaxHeight = 320,
                    VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
                    Content = _items,
                },
            },
        };

        MouseLeftButtonUp += (_, _) =>
        {
            if (_choices.Count == 0)
            {
                return;
            }
            // At least as wide as the button, so the list reads as belonging to it.
            ((Border)_popup.Child).MinWidth = ActualWidth;
            Rebuild();
            _popup.IsOpen = true;
        };

        Show();
    }

    /// <summary>Set what can be chosen, and what is chosen now.</summary>
    public void SetChoices(IReadOnlyList<(string Value, string Label)> choices, string? value)
    {
        _choices = choices;
        Value = value;
        Show();
    }

    private void Show()
    {
        var chosen = _choices.FirstOrDefault(c => c.Value == Value);
        _label.Text = chosen.Label ?? Placeholder;
        _label.Foreground = Res(chosen.Label is null ? "TextGhost" : "TextDim");
    }

    private void Rebuild()
    {
        _items.Children.Clear();

        foreach (var (value, label) in _choices)
        {
            var row = new Border
            {
                Height = 28,
                CornerRadius = new CornerRadius(6),
                Background = Brushes.Transparent,
                Cursor = Cursors.Hand,
                Child = new TextBlock
                {
                    Text = label,
                    Margin = new Thickness(9, 0, 9, 0),
                    VerticalAlignment = VerticalAlignment.Center,
                    FontWeight = value == Value ? FontWeights.Bold : FontWeights.Normal,
                    Foreground = Res(value == Value ? "Text" : "TextDim"),
                },
            };

            row.MouseEnter += (_, _) => row.Background = Res("LineStrong");
            row.MouseLeave += (_, _) => row.Background = Brushes.Transparent;
            row.MouseLeftButtonUp += (_, _) =>
            {
                _popup.IsOpen = false;
                Value = value;
                Show();
                Chosen?.Invoke(value);
            };

            _items.Children.Add(row);
        }
    }

    private static Brush Res(string key) => (Brush)Application.Current.Resources[key];
}
