using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Shapes;

namespace Lanes.Ui.Controls;

/// <summary>
/// An on/off switch, for the settings window.
/// </summary>
/// <remarks>
/// Hand-drawn for the same reason as the other controls here: the stock
/// <c>CheckBox</c> draws a light box that reads as borrowed furniture on this
/// window's near-black ground. The track uses the warning amber when on, which
/// is the only colour in the palette that is not a channel's.
/// </remarks>
public sealed class Toggle : Border
{
    private readonly Ellipse _knob = new() { Width = 14, Height = 14 };
    private bool _on;

    /// <summary>The user flipped it. Carries the new state.</summary>
    public event Action<bool>? Toggled;

    public Toggle()
    {
        Width = 36;
        Height = 20;
        CornerRadius = new CornerRadius(10);
        Padding = new Thickness(3);
        Cursor = Cursors.Hand;
        Child = _knob;

        MouseLeftButtonUp += (_, _) =>
        {
            IsOn = !IsOn;
            Toggled?.Invoke(IsOn);
        };

        Paint();
    }

    /// <summary>On or off. Setting it does not raise <see cref="Toggled"/>.</summary>
    public bool IsOn
    {
        get => _on;
        set
        {
            _on = value;
            Paint();
        }
    }

    private void Paint()
    {
        Background = (Brush)Application.Current.Resources[_on ? "Warn" : "Raised"];
        _knob.Fill = (Brush)Application.Current.Resources[_on ? "Ground" : "TextFaint"];
        _knob.HorizontalAlignment = _on ? HorizontalAlignment.Right : HorizontalAlignment.Left;
    }
}
