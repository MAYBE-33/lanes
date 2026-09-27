using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;

using Path = System.Windows.Shapes.Path;

namespace Lanes.Ui.Controls;

/// <summary>
/// The window's icons, drawn as vectors.
/// </summary>
/// <remarks>
/// <para>
/// <b>Why these are not emoji.</b> The characters 🔇 and 🔉 are the quickest
/// way to get a speaker on screen, and the wrong one. They come from Segoe UI
/// Emoji rather than the UI font, so they
/// carry their own weight, their own optical size and their own vertical
/// metrics — the glyph sits slightly off centre, ignores the colour it is
/// given on some builds, and changes shape with the Windows version. Next to
/// hand-drawn faders that reads as borrowed furniture.
/// </para>
/// <para>
/// These are a few lines of path data each, take the brush they are handed,
/// and scale with the DPI like everything else in the window.
/// </para>
/// <para>
/// All geometry is authored in a 16x16 box and scaled by the caller, so the
/// numbers below can be read as coordinates on a small grid.
/// </para>
/// </remarks>
internal static class Icons
{
    /// <summary>The speaker body and cone, filled.</summary>
    private static readonly Geometry SpeakerBody =
        Freeze(Geometry.Parse("M2,6.3 L4.7,6.3 L9.1,2.6 L9.1,13.4 L4.7,9.7 L2,9.7 Z"));

    /// <summary>The two arcs of sound, stroked. Only drawn when not muted.</summary>
    private static readonly Geometry SpeakerWaves = Freeze(Geometry.Parse(
        "M11.2,5.7 A3.2,3.2 0 0 1 11.2,10.3 M13.1,3.6 A5.8,5.8 0 0 1 13.1,12.4"));

    /// <summary>The cross that replaces the arcs when muted.</summary>
    private static readonly Geometry SpeakerCross =
        Freeze(Geometry.Parse("M11.4,6.1 L14.8,9.9 M14.8,6.1 L11.4,9.9"));

    /// <summary>
    /// A speaker, with arcs or with a cross.
    /// </summary>
    /// <remarks>
    /// Returned as a container of two paths rather than one, because the body
    /// is a fill and the arcs are a stroke — combining them into a single
    /// geometry would mean either outlining the body or filling the arcs.
    /// </remarks>
    public static Grid Speaker(bool muted, Brush brush, double size = 16)
    {
        var scale = size / 16.0;

        var body = new Path
        {
            Data = SpeakerBody,
            Fill = brush,
            Stretch = Stretch.None,
            HorizontalAlignment = HorizontalAlignment.Left,
            VerticalAlignment = VerticalAlignment.Top,
        };

        var mark = new Path
        {
            Data = muted ? SpeakerCross : SpeakerWaves,
            Stroke = brush,
            StrokeThickness = 1.5,
            StrokeStartLineCap = PenLineCap.Round,
            StrokeEndLineCap = PenLineCap.Round,
            Stretch = Stretch.None,
            HorizontalAlignment = HorizontalAlignment.Left,
            VerticalAlignment = VerticalAlignment.Top,
        };

        // A LayoutTransform, not a RenderTransform: the parent has to measure
        // the scaled size, or a 16-unit icon asked to be 13 would still claim
        // 16 units of the control it sits in.
        var host = new Grid
        {
            Width = 16,
            Height = 16,
            LayoutTransform = new ScaleTransform(scale, scale),
            IsHitTestVisible = false,
        };

        host.Children.Add(body);
        host.Children.Add(mark);
        return host;
    }

    /// <summary>Replace a speaker icon's arcs with a cross, or back.</summary>
    /// <remarks>
    /// So a mute toggle updates in place. Rebuilding the icon would be harmless
    /// here, but "update, never recreate" is the rule this window is built on
    /// and it is worth not having exceptions to explain.
    /// </remarks>
    public static void SetSpeaker(Grid icon, bool muted, Brush brush)
    {
        if (icon.Children.Count != 2
            || icon.Children[0] is not Path body
            || icon.Children[1] is not Path mark)
        {
            return;
        }

        body.Fill = brush;
        mark.Stroke = brush;
        mark.Data = muted ? SpeakerCross : SpeakerWaves;
    }

    public enum Direction
    {
        Up,
        Down,
        Left,
        Right,
    }

    /// <summary>
    /// A chevron, stroked.
    /// </summary>
    /// <remarks>
    /// The text character "⌄" was doing this job, and it sat wrong: its
    /// baseline put it low in the line box, which had been corrected with a
    /// negative top margin. A shape needs no such apology.
    /// </remarks>
    public static Path Chevron(Direction direction, Brush brush, double size = 10)
    {
        var data = direction switch
        {
            Direction.Up => "M1.4,6.4 L5,2.6 L8.6,6.4",
            Direction.Left => "M6.4,1.4 L2.6,5 L6.4,8.6",
            Direction.Right => "M3.6,1.4 L7.4,5 L3.6,8.6",
            _ => "M1.4,3.6 L5,7.4 L8.6,3.6",
        };

        return new Path
        {
            Data = Freeze(Geometry.Parse(data)),
            Stroke = brush,
            StrokeThickness = 1.4,
            StrokeStartLineCap = PenLineCap.Round,
            StrokeEndLineCap = PenLineCap.Round,
            StrokeLineJoin = PenLineJoin.Round,
            Stretch = Stretch.None,
            LayoutTransform = new ScaleTransform(size / 10.0, size / 10.0),
            Width = 10,
            Height = 10,
            IsHitTestVisible = false,
        };
    }

    private static Geometry Freeze(Geometry geometry)
    {
        geometry.Freeze();
        return geometry;
    }
}
