using System.Windows;
using System.Windows.Input;
using System.Windows.Media;

namespace Lanes.Ui.Controls;

/// <summary>
/// The Game/Chat balance: one horizontal control sitting over the two strips
/// it governs.
/// </summary>
/// <remarks>
/// <para>
/// Centre leaves both channels exactly as their own faders set them. Sliding
/// toward one end quietens the <i>other</i> channel and leaves the favoured one
/// alone, so each fader keeps its meaning and nothing is ever boosted above
/// what the user set. The arithmetic lives in the core — see
/// <c>config::ChatMix</c> — and this control only reports a position.
/// </para>
/// <para>
/// <b>It behaves exactly as a channel fader does</b>, because it shares the
/// same <see cref="Detents"/>: the same five positions, the same snap radius,
/// the same dwell that switches the magnetism off once you have clearly chosen
/// a value, the same clickable paddles, the same grip. It is a fader lying on
/// its side, and the only reason it is a separate control is that its travel
/// runs −1 to +1 about a meaningful centre.
/// </para>
/// </remarks>
public sealed class ChatMixSlider : FrameworkElement
{
    private const double Step = 0.01;

    /// <summary>
    /// How long to keep showing our own value after release.
    /// </summary>
    /// <remarks>
    /// The same problem the faders have. Handing the display straight back to
    /// whatever the core last said on release would jump the handle back to a
    /// stale value until the core's answer arrived, and the snap to centre
    /// would be invisible - settled correctly, and looking as if it had not.
    /// </remarks>
    private static readonly TimeSpan SettleTimeout = TimeSpan.FromSeconds(2);

    private const double TrackHeight = 6;

    /// <summary>Half the handle's width, and the travel inset at each end.</summary>
    private const double HandleRadius = 9;

    private const double HandleWidth = 15;
    private const double HandleHeight = 21;

    /// <summary>
    /// How tall the control is, which the detent paddles set.
    /// </summary>
    /// <remarks>
    /// The track sits in the upper part and the paddle heads hang below it, the
    /// way a fader's heads sit to the left of its track. Tall enough that a
    /// head clears the grip when the handle is parked on that detent — at 34,
    /// which is what this was before the detents arrived, the two overlapped.
    /// </remarks>
    private const double ControlHeight = 52;

    /// <summary>Where the track's centre line sits within the control.</summary>
    private const double TrackY = 20;

    public static readonly DependencyProperty ValueProperty = DependencyProperty.Register(
        nameof(Value), typeof(double), typeof(ChatMixSlider),
        new FrameworkPropertyMetadata(
            0.0,
            FrameworkPropertyMetadataOptions.AffectsRender,
            (d, e) => ((ChatMixSlider)d).OnCoreValue((double)e.NewValue)));

    /// <summary>-1 (all toward the left channel) to +1 (all toward the right).</summary>
    public double Value
    {
        get => (double)GetValue(ValueProperty);
        set => SetValue(ValueProperty, value);
    }

    public Brush LeftAccent { get; set; } = Brushes.Gray;
    public Brush RightAccent { get; set; } = Brushes.Gray;

    public event EventHandler<double>? ValueChanged;

    private bool _dragging;
    private bool _settling;
    private bool _deliberate;
    private DateTime _steadySince = DateTime.UtcNow;
    private int _hoveredTab = -1;
    private double _live;
    private double _sent;
    private DateTime _sentAt;

    private double Displayed
    {
        get
        {
            if (_dragging)
            {
                return _live;
            }

            if (_settling && DateTime.UtcNow - _sentAt > SettleTimeout)
            {
                _settling = false;
            }

            return _settling ? _live : Math.Clamp(Value, -1, 1);
        }
    }

    /// <summary>The core reported a value. Stop ignoring it once it agrees.</summary>
    private void OnCoreValue(double value)
    {
        if (_settling && Math.Abs(value - _sent) < Step / 2)
        {
            _settling = false;
        }
    }

    public ChatMixSlider()
    {
        Height = ControlHeight;
        Cursor = Cursors.Hand;
        Focusable = true;
    }

    /// <summary>
    /// Only the track row and the detent paddles are live.
    /// </summary>
    /// <remarks>
    /// The same rule the faders follow. Without it the element is a rectangle
    /// and every pixel responds, including the empty space between paddles —
    /// click there and the balance jumps. Done here rather than in the mouse
    /// handlers so the cursor is right too, and so a click in a gap passes
    /// through to the card underneath.
    /// </remarks>
    protected override HitTestResult? HitTestCore(PointHitTestParameters parameters)
    {
        var point = parameters.HitPoint;
        var live = TrackRow.Contains(point) || DetentAt(point) >= 0;
        return live ? new PointHitTestResult(this, point) : null;
    }

    // --- Geometry ----------------------------------------------------------

    /// <summary>The band the track and the grip occupy.</summary>
    private Rect TrackRow => new(0, TrackY - HandleHeight / 2, Math.Max(1, ActualWidth), HandleHeight);

    /// <summary>Distance from the track's centre line to a paddle head's outer edge.</summary>
    private static double HeadOuter => ControlHeight - Detents.EdgeInset - TrackY;

    /// <summary>The x of a position given as a fraction of travel.</summary>
    private double XOfFraction(double fraction) =>
        HandleRadius + fraction * Math.Max(1, ActualWidth - 2 * HandleRadius);

    private double XOfValue(double value) => XOfFraction((Math.Clamp(value, -1, 1) + 1) / 2);

    private static double ToFraction(double value) => (Math.Clamp(value, -1, 1) + 1) / 2;

    private static double ToValue(double fraction) => Math.Clamp(fraction, 0, 1) * 2 - 1;

    private double FractionAt(double x)
    {
        var travel = Math.Max(1, ActualWidth - 2 * HandleRadius);
        return Math.Clamp((x - HandleRadius) / travel, 0, 1);
    }

    private double ValueAt(double x) => Quantise(ToValue(FractionAt(x)));

    private static double Quantise(double v) => Math.Round(v / Step) * Step;

    /// <summary>
    /// Placement for one paddle: a quarter turn, then onto the detent.
    /// </summary>
    /// <remarks>
    /// <see cref="Detents.Paddle"/> is built about the origin with the head on
    /// negative x. Rotating −90° maps +x to up, so the stem runs up through the
    /// track and the head hangs below it — which is the horizontal analogue of
    /// the fader's head-to-the-left.
    /// </remarks>
    private Transform PaddleTransform(double fraction)
    {
        var group = new TransformGroup();
        group.Children.Add(new RotateTransform(-90));
        group.Children.Add(new TranslateTransform(XOfFraction(fraction), TrackY));
        group.Freeze();
        return group;
    }

    /// <summary>Which detent a point falls on, or -1.</summary>
    private int DetentAt(Point point)
    {
        var box = Detents.HeadHitBox(HeadOuter);

        for (var index = 0; index < Detents.Positions.Length; index++)
        {
            // Into the paddle's own frame, which is the inverse of the rotation
            // used to draw it: undo the translation, then the quarter turn.
            var dx = point.X - XOfFraction(Detents.Positions[index]);
            var dy = point.Y - TrackY;
            var local = new Point(-dy, dx);

            if (box.Contains(local))
            {
                return index;
            }
        }

        return -1;
    }

    // --- Input -------------------------------------------------------------

    protected override void OnMouseLeftButtonDown(MouseButtonEventArgs e)
    {
        base.OnMouseLeftButtonDown(e);

        var position = e.GetPosition(this);

        // A paddle is a shortcut, not the start of a gesture: it sets the
        // balance and that is the whole interaction. Starting a drag from it as
        // well would mean the smallest wobble dragged the value straight back
        // off what was just asked for.
        var tab = DetentAt(position);
        if (tab >= 0)
        {
            _live = ToValue(Detents.Positions[tab]);
            _dragging = false;
            _settling = true;
            _deliberate = true;
            Focus();
            Publish();
            e.Handled = true;
            return;
        }

        _dragging = true;
        _settling = false;
        _deliberate = false;
        _steadySince = DateTime.UtcNow;
        _live = ValueAt(position.X);
        CaptureMouse();
        Focus();
        Publish();
        e.Handled = true;
    }

    protected override void OnMouseMove(MouseEventArgs e)
    {
        base.OnMouseMove(e);

        if (!_dragging)
        {
            var over = DetentAt(e.GetPosition(this));
            if (over != _hoveredTab)
            {
                _hoveredTab = over;
                InvalidateVisual();
            }
            return;
        }

        var next = ValueAt(e.GetPosition(this).X);

        if (Math.Abs(next - _live) < Step / 2)
        {
            // Tiny movements that do not change the value still count as
            // holding still, so a hand that is not quite steady is not
            // penalised for it. The decision itself is made at release.
            if (!_deliberate && DateTime.UtcNow - _steadySince >= Detents.DwellToCommit)
            {
                _deliberate = true;
            }
            return;
        }

        _live = next;
        _steadySince = DateTime.UtcNow;
        Publish();
    }

    protected override void OnMouseLeftButtonUp(MouseButtonEventArgs e)
    {
        base.OnMouseLeftButtonUp(e);

        if (!_dragging)
        {
            return;
        }

        // Read here, not during the drag. A pointer held genuinely still
        // produces no move events at all, so a check in the move handler never
        // runs — it works for a hand with a tremor and fails for a steady one.
        if (!_deliberate && DateTime.UtcNow - _steadySince >= Detents.DwellToCommit)
        {
            _deliberate = true;
        }

        if (!_deliberate)
        {
            _live = ToValue(Detents.Snap(ToFraction(_live)));
        }

        _dragging = false;

        // Keep showing the settled value until the core confirms it, rather
        // than handing the display back to a stale echo.
        _settling = true;

        ReleaseMouseCapture();
        Publish();
        e.Handled = true;
    }

    protected override void OnMouseLeave(MouseEventArgs e)
    {
        base.OnMouseLeave(e);
        if (_hoveredTab != -1)
        {
            _hoveredTab = -1;
            InvalidateVisual();
        }
    }

    protected override void OnMouseWheel(MouseWheelEventArgs e)
    {
        base.OnMouseWheel(e);

        var next = Quantise(Math.Clamp(Displayed + Math.Sign(e.Delta) * Step, -1, 1));
        if (Math.Abs(next - Displayed) < Step / 2)
        {
            return;
        }

        _live = next;
        _settling = true;
        Publish();
        e.Handled = true;
    }

    protected override void OnKeyDown(KeyEventArgs e)
    {
        base.OnKeyDown(e);

        var step = e.Key switch
        {
            Key.Left or Key.Down => -Step,
            Key.Right or Key.Up => Step,
            Key.PageDown => -0.1,
            Key.PageUp => 0.1,
            _ => 0.0,
        };

        if (e.Key == Key.Home)
        {
            _live = 0;
        }
        else if (step != 0)
        {
            _live = Quantise(Math.Clamp(Displayed + step, -1, 1));
        }
        else
        {
            return;
        }

        _settling = true;
        Publish();
        e.Handled = true;
    }

    private void Publish()
    {
        InvalidateVisual();
        _sent = _live;
        _sentAt = DateTime.UtcNow;
        ValueChanged?.Invoke(this, _live);
    }

    // --- Drawing -----------------------------------------------------------

    protected override void OnRender(DrawingContext dc)
    {
        var width = ActualWidth;
        if (width <= 0 || ActualHeight <= 0)
        {
            return;
        }

        var theme = Application.Current.Resources;
        var radius = TrackHeight / 2;

        // Hit-testable, but only where something can be clicked. See HitTestCore.
        dc.DrawRectangle(Brushes.Transparent, null, TrackRow);

        dc.DrawRoundedRectangle(
            (Brush)theme["Track"], null,
            new Rect(HandleRadius, TrackY - radius, Math.Max(1, width - 2 * HandleRadius), TrackHeight),
            radius, radius);

        var value = Displayed;
        var centre = XOfValue(0);
        var handle = XOfValue(value);
        var favoured = value > 0 ? RightAccent : LeftAccent;

        // The bar between centre and the handle is drawn in the accent of the
        // channel being FAVOURED, which is the one not being attenuated.
        if (Math.Abs(value) > 0.001)
        {
            var brush = favoured.Clone();
            brush.Opacity = 0.75;
            brush.Freeze();
            var from = Math.Min(centre, handle);
            var to = Math.Max(centre, handle);
            dc.DrawRoundedRectangle(brush, null, new Rect(from, TrackY - radius, to - from, TrackHeight), radius, radius);
        }

        var neutral = (Brush)theme["TextFaint"];
        var idle = (Brush)theme["DetentIdle"];
        var indexColour = Math.Abs(value) > 0.001 ? favoured : neutral;

        // Paddles, the same shape the faders use.
        for (var index = 0; index < Detents.Positions.Length; index++)
        {
            var fraction = Detents.Positions[index];
            var onIt = Math.Abs(ToFraction(value) - fraction) < Step / 4;
            var hot = index == _hoveredTab;

            Brush fill;
            if (onIt)
            {
                // Centre is the position that attenuates nothing, so it stays
                // neutral even when the handle is parked on it. Colouring it
                // would claim a channel is being favoured when none is.
                fill = indexColour;
            }
            else if (hot)
            {
                var brush = (fraction < 0.5 ? LeftAccent : fraction > 0.5 ? RightAccent : neutral).Clone();
                brush.Opacity = 0.55;
                brush.Freeze();
                fill = brush;
            }
            else
            {
                fill = idle;
            }

            dc.DrawGeometry(fill, null, Detents.Paddle(HeadOuter, radius, PaddleTransform(fraction)));
        }

        // The grip, matching the faders. See CapPaint.
        var rect = new Rect(
            handle - HandleWidth / 2,
            TrackY - HandleHeight / 2,
            HandleWidth,
            HandleHeight);

        dc.DrawRoundedRectangle(CapPaint.Face, CapPaint.Edge, rect, 5, 5);

        var x = Math.Round(handle);
        dc.DrawLine(
            CapPaint.Index(indexColour),
            new Point(x, rect.Top + 5),
            new Point(x, rect.Bottom - 5));
    }
}
