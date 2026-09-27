using System.Globalization;
using System.Windows;
using System.Windows.Input;
using System.Windows.Media;

namespace Lanes.Ui.Controls;

/// <summary>
/// A vertical fader with the level meter drawn behind the track.
/// </summary>
/// <remarks>
/// <para>
/// <b>Why this is drawn by hand rather than being a styled <c>Slider</c>.</b>
/// Three of its behaviours — the meter fill <i>inside</i> the track, detents with magnetism, and a handle that never
/// leaves the pointer during a drag — all fight the stock control's template
/// and its value coercion. Drawing it is less code than bending it.
/// </para>
/// <para>
/// <b>The bug this control exists to not have.</b> Every drag step sends a
/// command and the core echoes the resulting state back. A control that simply
/// displayed the echo would fight the hand holding it - and one rebuilt on each
/// echo would be destroyed mid-drag, roughly once per step.
/// </para>
/// <para>
/// So while the pointer is down this control shows <i>its own</i> value and
/// ignores anything arriving from the core. The core still receives every step
/// and remains the source of truth the moment the drag ends. Any UI that binds
/// a slider straight to a value echoed back by a server has this bug; the fix
/// is not framework-specific.
/// </para>
/// </remarks>
public sealed class Fader : FrameworkElement
{
    // --- Tuning ------------------------------------------------------------

    /// <summary>The smallest step the user can set: 1%.</summary>
    private const double Step = 0.01;

    // The detent scale, the magnetism and the paddle's proportions live in
    // `Detents`, shared with the Game/Chat mix so the two behave identically.
    // Two copies of those numbers would agree today and drift the first time
    // one of them was tuned.

    private const double TrackWidth = 6;

    /// <summary>Half the cap's height, and the travel inset at each end.</summary>
    private const double HandleRadius = 8;

    /// <summary>The cap, drawn like a real fader's: a grip, not a dot.</summary>
    ///
    /// Sized so it clears the detent heads. At 26 wide in a 68-wide element the
    /// cap's edge landed right against them and the two read as one cluttered
    /// shape whenever the channel sat on a quarter.
    private const double CapWidth = 26;
    private const double CapHeight = 14;

    // --- Properties --------------------------------------------------------

    public static readonly DependencyProperty ValueProperty = DependencyProperty.Register(
        nameof(Value), typeof(double), typeof(Fader),
        new FrameworkPropertyMetadata(
            1.0,
            FrameworkPropertyMetadataOptions.AffectsRender,
            (d, e) => ((Fader)d).OnCoreValue((double)e.NewValue)));

    /// <summary>The level the core last reported, 0 to 1.</summary>
    public double Value
    {
        get => (double)GetValue(ValueProperty);
        set => SetValue(ValueProperty, value);
    }

    public static readonly DependencyProperty MeterProperty = DependencyProperty.Register(
        nameof(Meter), typeof(double), typeof(Fader),
        // Deliberately not AffectsRender: the drawn level is eased towards this
        // one every frame rather than snapping to it. See `_meterShown`.
        new FrameworkPropertyMetadata(0.0));

    /// <summary>
    /// The level the core last reported: a linear peak, 0 to 1.
    /// </summary>
    /// <remarks>
    /// Arrives about thirty times a second. What is drawn is derived from it
    /// rather than equal to it — see <see cref="MeterShown"/> for why both the
    /// timing and the scale need work before it looks like a level.
    /// </remarks>
    public double Meter
    {
        get => (double)GetValue(MeterProperty);
        set => SetValue(MeterProperty, value);
    }

    // --- Meter ballistics --------------------------------------------------

    /// <summary>The level actually drawn, eased towards <see cref="Meter"/>.</summary>
    private double _meterShown;

    /// <summary>The meter's gradient, rebuilt only when the accent changes.</summary>
    /// <remarks>
    /// <c>OnRender</c> runs whenever the drawn meter level moves, which while
    /// audio is playing is every frame. Brushes built in there would be
    /// allocated and frozen sixty times a second per strip for no reason.
    /// </remarks>
    private Brush? _meterBrush;
    private Brush? _meterPeak;
    private Brush? _meterFor;

    private DateTime _lastFrame = DateTime.UtcNow;

    /// <summary>
    /// How quickly the bar falls, as a time constant in seconds.
    /// </summary>
    /// <remarks>
    /// A peak meter rises instantly and falls slowly: that is what makes it
    /// readable, because a transient you cannot see is a transient you cannot
    /// act on. Rising is therefore immediate and only the fall is eased.
    ///
    /// This is also what hides the gap between samples. Thirty a second is
    /// plenty of information but it is not a smooth picture on its own, and the
    /// previous ten a second read as a stutter — the user's words were "running
    /// at 2 frames a second". Easing between them at the display's own rate is
    /// what turns samples into motion.
    /// </remarks>
    private const double DecaySeconds = 0.18;

    /// <summary>
    /// The quietest level the bar shows, in decibels below full scale.
    /// </summary>
    /// <remarks>
    /// The level is a linear peak, which is the honest number for the API to
    /// carry and a poor one to draw. Normal speech peaks around 0.1-0.3 linear,
    /// so a bar drawn straight from it barely leaves the floor — the meter
    /// looks broken while the audio is plainly audible. Every real meter is
    /// logarithmic for this reason.
    ///
    /// At -60dB, 0.1 linear draws at two thirds height and 0.01 at a third,
    /// which is what a level meter is supposed to look like.
    /// </remarks>
    private const double FloorDb = -60.0;

    /// <summary>The eased level, on a decibel scale, ready to draw.</summary>
    private double MeterShown => ToDisplayScale(_meterShown);

    private static double ToDisplayScale(double linear)
    {
        if (linear <= 0)
        {
            return 0;
        }

        var db = 20.0 * Math.Log10(Math.Clamp(linear, 0.0, 1.0));
        return Math.Clamp((db - FloorDb) / -FloorDb, 0.0, 1.0);
    }

    /// <summary>
    /// Ease the drawn level towards the reported one, once per displayed frame.
    /// </summary>
    /// <remarks>
    /// Subscribed only while this control is loaded, and it redraws nothing
    /// once the two agree — so a silent channel costs a comparison per frame
    /// and no rendering at all.
    /// </remarks>
    private void OnFrame(object? sender, EventArgs e)
    {
        var now = DateTime.UtcNow;
        var elapsed = (now - _lastFrame).TotalSeconds;
        _lastFrame = now;

        var target = Math.Clamp(Meter, 0.0, 1.0);
        var previous = _meterShown;

        if (target >= _meterShown)
        {
            _meterShown = target;
        }
        else
        {
            // Exponential decay, framed in elapsed time rather than in frames,
            // so it falls at the same speed whatever the display is doing.
            _meterShown = target + (_meterShown - target) * Math.Exp(-elapsed / DecaySeconds);
            if (_meterShown < 0.0005)
            {
                _meterShown = 0;
            }
        }

        if (Math.Abs(_meterShown - previous) > 0.0002)
        {
            InvalidateVisual();
        }
    }

    public static readonly DependencyProperty CeilingProperty = DependencyProperty.Register(
        nameof(Ceiling), typeof(double), typeof(Fader),
        new FrameworkPropertyMetadata(1.0, FrameworkPropertyMetadataOptions.AffectsRender));

    /// <summary>
    /// The highest level this channel may reach, 0 to 1.
    /// </summary>
    /// <remarks>
    /// Master is a ceiling on every playback channel rather than a scalar on
    /// it, so a channel cannot be dragged above wherever Master currently sits.
    /// The handle stops there, which is the plainest way to show a limit: the
    /// alternative is letting it travel and then snapping back, which looks
    /// like a fault rather than a rule.
    ///
    /// Drawn as a faint line across the track when it is below full, so the
    /// stop is never a mystery.
    /// </remarks>
    public double Ceiling
    {
        get => (double)GetValue(CeilingProperty);
        set => SetValue(CeilingProperty, value);
    }

    public static readonly DependencyProperty MutedProperty = DependencyProperty.Register(
        nameof(Muted), typeof(bool), typeof(Fader),
        new FrameworkPropertyMetadata(false, FrameworkPropertyMetadataOptions.AffectsRender));

    public bool Muted
    {
        get => (bool)GetValue(MutedProperty);
        set => SetValue(MutedProperty, value);
    }

    public static readonly DependencyProperty AccentProperty = DependencyProperty.Register(
        nameof(Accent), typeof(Brush), typeof(Fader),
        new FrameworkPropertyMetadata(Brushes.Gray, FrameworkPropertyMetadataOptions.AffectsRender));

    public Brush Accent
    {
        get => (Brush)GetValue(AccentProperty);
        set => SetValue(AccentProperty, value);
    }

    /// <summary>
    /// The user moved the fader. Raised for every step of a drag, not only at
    /// the end, so the core follows the gesture live.
    /// </summary>
    public event EventHandler<double>? ValueChanged;

    // --- Drag state --------------------------------------------------------

    private bool _dragging;

    /// <summary>
    /// True after a release, until the core confirms the value we sent it.
    /// </summary>
    /// <remarks>
    /// This is the half that was missing, and it is what made the fader look
    /// broken even though every value was correct.
    ///
    /// A drag sends a command per step. Each one makes the core write its
    /// config and re-check every audio session, so the replies run seconds
    /// behind a quick gesture. Letting go handed the display back to those
    /// replies — so the handle jumped to wherever the backlog had reached and
    /// then walked forward through every 1% the user had passed through. A
    /// replay of the drag, after the drag.
    ///
    /// So the handle stays where it was put and ignores the echoes until one of
    /// them agrees with it.
    /// </remarks>
    private bool _settling;

    private double _live;

    /// <summary>Which detent tab the pointer is over, or -1.</summary>
    private int _hoveredTab = -1;

    /// <summary>When the pointer last settled on the value it is showing.</summary>
    private DateTime _steadySince = DateTime.UtcNow;

    /// <summary>
    /// True once the gesture has dwelt long enough to count as deliberate.
    /// </summary>
    /// <remarks>
    /// Latched rather than recomputed at release, so that pausing on a value
    /// and then nudging it by a percent still counts — the user has clearly
    /// stopped sweeping and started aiming.
    /// </remarks>
    private bool _deliberate;

    /// <summary>The last value handed to the core, which is what to wait for.</summary>
    private double _sent;

    /// <summary>When it was sent, so a lost reply cannot freeze the control.</summary>
    private DateTime _sentAt;

    /// <summary>
    /// How long to keep ignoring the core before giving up and trusting it.
    /// </summary>
    /// <remarks>
    /// A safety net, not a timing assumption. If a command is dropped or the
    /// core is busy, the fader must return to showing real state rather than a
    /// value nothing agreed to.
    /// </remarks>
    private static readonly TimeSpan SettleTimeout = TimeSpan.FromSeconds(2);

    /// <summary>
    /// The shortest gap between commands while dragging.
    /// </summary>
    /// <remarks>
    /// A drag across the full travel passes through a hundred 1% steps. Sending
    /// each one gave the core a hundred config writes and a hundred session
    /// sweeps to work through, which is what put the replies so far behind.
    /// At 45ms a fast drag sends about a dozen, and the release always sends the
    /// final value, so nothing is lost by dropping the ones in between.
    /// </remarks>
    private static readonly TimeSpan SendInterval = TimeSpan.FromMilliseconds(45);

    private DateTime _lastSend = DateTime.MinValue;

    /// <summary>What to draw: the user's value while they own it, else the core's.</summary>
    public double Displayed
    {
        get
        {
            if (_dragging)
            {
                return _live;
            }

            // Checked here as well as when a value arrives, because if the
            // connection has dropped no value ever arrives — and a fader that
            // waits for a confirmation that can never come would be stuck
            // showing a number nothing agreed to.
            if (_settling && DateTime.UtcNow - _sentAt > SettleTimeout)
            {
                _settling = false;
            }

            return _settling ? _live : Clamp(Value);
        }
    }

    /// <summary>
    /// Only the detents and the track column are live.
    /// </summary>
    /// <remarks>
    /// Without this the element is a rectangle and every pixel of it responds,
    /// including the empty space between detents — click there and the volume
    /// jumps. Restricting it here rather than in the mouse handlers also gives
    /// the right cursor and lets clicks in the gaps pass through to the strip.
    /// </remarks>
    protected override HitTestResult? HitTestCore(PointHitTestParameters parameters)
    {
        var point = parameters.HitPoint;
        var live = TrackColumn.Contains(point) || DetentAt(point) >= 0;
        return live ? new PointHitTestResult(this, point) : null;
    }

    public Fader()
    {
        Focusable = true;
        Cursor = Cursors.Hand;

        Loaded += (_, _) =>
        {
            _lastFrame = DateTime.UtcNow;
            System.Windows.Media.CompositionTarget.Rendering += OnFrame;
        };
        Unloaded += (_, _) => System.Windows.Media.CompositionTarget.Rendering -= OnFrame;
    }

    /// <summary>The core reported a value. Stop ignoring it once it agrees.</summary>
    private void OnCoreValue(double value)
    {
        if (!_settling)
        {
            return;
        }

        // Half a step of tolerance: the value has been through a JSON round trip
        // as a float, so exact equality is not something to rely on.
        if (Math.Abs(value - _sent) < Step / 2 || DateTime.UtcNow - _sentAt > SettleTimeout)
        {
            _settling = false;
        }
    }

    // --- Input -------------------------------------------------------------

    protected override void OnMouseLeftButtonDown(MouseButtonEventArgs e)
    {
        base.OnMouseLeftButtonDown(e);

        var position = e.GetPosition(this);

        // A tab is a shortcut, not the start of a gesture: it sets the level
        // and that is the whole interaction. Starting a drag from it as well
        // would mean the smallest wobble dragged the value straight back off
        // the value that was just asked for.
        var tab = DetentAt(position);
        if (tab >= 0)
        {
            _live = Detents.Positions[tab];
            _dragging = false;
            _settling = true;
            _deliberate = true;
            Focus();
            Publish(force: true);
            e.Handled = true;
            return;
        }

        _dragging = true;
        _settling = false;
        _deliberate = false;
        _steadySince = DateTime.UtcNow;
        _live = ValueAt(position.Y);
        CaptureMouse();
        Focus();
        Publish(force: true);
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
                Cursor = over >= 0 ? Cursors.Hand : Cursors.Hand;
                InvalidateVisual();
            }
            return;
        }

        var next = ValueAt(e.GetPosition(this).Y);

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
        Publish(force: false);
    }

    protected override void OnMouseLeftButtonUp(MouseButtonEventArgs e)
    {
        base.OnMouseLeftButtonUp(e);

        if (!_dragging)
        {
            return;
        }

        // The magnetism happens here rather than during the drag, so the handle
        // tracks the pointer honestly and only settles when let go. Pulling it
        // mid-drag makes the control feel like it is fighting you.
        //
        // And it is skipped entirely when the gesture dwelt on a value: at that
        // point the user has told us what they want and the detent is no longer
        // a help. See `Detents.DwellToCommit`.
        // Evaluated HERE and not only during the drag, which is the whole
        // reason it works.
        //
        // Not in the mouse-move handler: a pointer that is genuinely held still
        // produces no move events at all, so a check there would never run. It
        // would appear to work for a hand with a slight tremor and fail for a
        // steady one, which is the worst kind of intermittent.
        //
        // The time since the value last changed is the dwell, and it can always
        // be read at release.
        if (!_deliberate && DateTime.UtcNow - _steadySince >= Detents.DwellToCommit)
        {
            _deliberate = true;
        }

        if (!_deliberate)
        {
            _live = Detents.Snap(_live);
        }
        _dragging = false;

        // The handle keeps showing this until the core agrees, rather than
        // handing the display straight back to a queue of stale replies.
        _settling = true;

        ReleaseMouseCapture();
        Publish(force: true);
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

        var direction = Math.Sign(e.Delta);
        var next = Quantise(Clamp(Displayed + direction * Step));
        if (Math.Abs(next - Displayed) < Step / 2)
        {
            return;
        }

        _live = next;
        _settling = true;
        Publish(force: true);
        e.Handled = true;
    }

    protected override void OnKeyDown(KeyEventArgs e)
    {
        base.OnKeyDown(e);

        var delta = e.Key switch
        {
            Key.Up or Key.Right => Step,
            Key.Down or Key.Left => -Step,
            Key.PageUp => 0.1,
            Key.PageDown => -0.1,
            _ => 0.0,
        };

        if (delta == 0.0)
        {
            return;
        }

        _live = Quantise(Clamp(Displayed + delta));
        _settling = true;
        Publish(force: true);
        e.Handled = true;
    }

    /// <summary>
    /// Redraw, and tell the core — but not on every single step.
    /// </summary>
    /// <param name="force">
    /// Bypass the throttle. Always true for the end of a gesture, so the value
    /// the user actually chose is never the one that got dropped.
    /// </param>
    private void Publish(bool force)
    {
        // The handle follows the pointer on every step regardless. Only the
        // command to the core is rationed.
        InvalidateVisual();

        var now = DateTime.UtcNow;
        if (!force && now - _lastSend < SendInterval)
        {
            return;
        }

        _lastSend = now;
        _sent = _live;
        _sentAt = now;
        ValueChanged?.Invoke(this, _live);
    }

    // --- Geometry ----------------------------------------------------------

    /// <summary>The value a pointer at <paramref name="y"/> represents.</summary>
    /// <remarks>
    /// The handle's centre travels between one radius from each end, so the
    /// extremes are reachable without half the handle hanging off the control.
    /// </remarks>
    private double ValueAt(double y)
    {
        var travel = Math.Max(1, ActualHeight - 2 * HandleRadius);
        var fraction = 1.0 - (y - HandleRadius) / travel;
        // Clamped to the ceiling, so the handle stops where Master does.
        return Math.Min(Quantise(Clamp(fraction)), Quantise(Clamp(Ceiling)));
    }

    /// <summary>The left edge of the track, which everything else hangs off.</summary>
    private double TrackLeft => (ActualWidth - TrackWidth) / 2;

    /// <summary>Distance from the track's centre to a paddle head's outer edge.</summary>
    private double HeadOuter => ActualWidth / 2 - Detents.EdgeInset;

    /// <summary>
    /// Placement for one paddle.
    /// </summary>
    /// <remarks>
    /// No rotation: <see cref="Detents.Paddle"/> is built in exactly this
    /// orientation, head on negative x, and it is the horizontal mix that has
    /// to turn it a quarter turn.
    /// </remarks>
    private Transform PaddleTransform(double detent)
    {
        var transform = new TranslateTransform(ActualWidth / 2, CentreOf(detent));
        transform.Freeze();
        return transform;
    }

    /// <summary>The column the cap and the track occupy.</summary>
    private Rect TrackColumn =>
        new((ActualWidth - CapWidth) / 2, 0, CapWidth, ActualHeight);

    /// <summary>Which detent a point falls on, or -1.</summary>
    /// <remarks>
    /// The <b>drawn shape</b> plus a few pixels, and nothing else. Anything
    /// looser makes the gaps between detents live, which is a control that
    /// changes the volume when you click somewhere you were not aiming.
    /// </remarks>
    private int DetentAt(Point point)
    {
        var box = Detents.HeadHitBox(HeadOuter);

        for (var index = 0; index < Detents.Positions.Length; index++)
        {
            // Into the paddle's own frame: undo the translation used to draw
            // it. Head and shoulder only - the stem crosses the track, and a
            // click there belongs to the track.
            var local = new Point(
                point.X - ActualWidth / 2,
                point.Y - CentreOf(Detents.Positions[index]));

            if (box.Contains(local))
            {
                return index;
            }
        }

        return -1;
    }

    private double CentreOf(double value) =>
        HandleRadius + (1.0 - value) * Math.Max(1, ActualHeight - 2 * HandleRadius);

    private static double Clamp(double v) => Math.Clamp(double.IsFinite(v) ? v : 0.0, 0.0, 1.0);

    private static double Quantise(double v) => Math.Round(v / Step) * Step;

    // Snapping is Detents.Snap: a fader's value IS its fraction of travel,
    // so no conversion is needed here. The mix, whose travel runs -1 to +1,
    // converts on the way in and out.

    // --- Drawing -----------------------------------------------------------

    protected override void OnRender(DrawingContext dc)
    {
        var width = ActualWidth;
        var height = ActualHeight;
        if (width <= 0 || height <= 0)
        {
            return;
        }

        var theme = Application.Current.Resources;
        var trackBrush = (Brush)theme["Track"];
        var idleBrush = (Brush)theme["DotIdle"];
        var faintBrush = (Brush)theme["TextFaint"];
        var detentBrush = (Brush)theme["DetentIdle"];

        var value = Displayed;
        var left = TrackLeft;
        var radius = TrackWidth / 2;

        // Hit-testable, but only where something can be clicked. See HitTestCore.
        dc.DrawRectangle(Brushes.Transparent, null, TrackColumn);

        dc.DrawRoundedRectangle(trackBrush, null, new Rect(left, 0, TrackWidth, height), radius, radius);

        // The meter is a fill INSIDE the track, so level and position read as
        // one element rather than two things to compare.
        if (!Muted && MeterShown > 0)
        {
            EnsureMeterBrushes();

            var fill = MeterShown * height;
            var top = height - fill;

            dc.DrawRoundedRectangle(
                _meterBrush, null,
                new Rect(left, top, TrackWidth, fill),
                radius, radius);

            // A brighter line along the top of the fill.
            //
            // A flat bar of colour is a coloured column; the same bar with a
            // lit leading edge is unmistakably a level. It also gives the eye
            // something precise to follow, which a soft gradient does not: a
            // solid block of colour reads as decoration rather than as
            // information.
            if (fill > 3)
            {
                dc.DrawRoundedRectangle(_meterPeak, null, new Rect(left, top, TrackWidth, 2), 1, 1);
            }
        }

        // The ceiling Master imposes, when there is one.
        var ceiling = Clamp(Ceiling);
        if (ceiling < 0.999)
        {
            var ceilingPen = new Pen(faintBrush, 1) { DashStyle = new DashStyle([2, 2], 0) };
            ceilingPen.Freeze();
            var y = Math.Round(CentreOf(ceiling)) + 0.5;
            dc.DrawLine(ceilingPen, new Point(left - 5, y), new Point(left + TrackWidth + 5, y));
        }

        // Detent paddles.
        for (var index = 0; index < Detents.Positions.Length; index++)
        {
            var detent = Detents.Positions[index];
            var hot = index == _hoveredTab;

            // "Active" is the channel actually sitting on this quarter, shown in
            // the channel's own colour. It is the
            // one place a detent earns an accent; a hovered one is a lighter
            // version of the same idea, so the two never compete.
            var onIt = Math.Abs(value - detent) < Step / 2;

            Brush fill;
            if (onIt)
            {
                fill = Muted ? idleBrush : Accent;
            }
            else if (hot)
            {
                fill = HoverFill(Accent);
            }
            else
            {
                fill = detentBrush;
            }

            // No outline. A one-pixel stroke around a fill of almost the same
            // value is what made these look muddy rather than crisp: the two
            // edges fight and the shape loses its silhouette. A single flat
            // fill against the card reads cleanly at this size.
            dc.DrawGeometry(fill, null, Detents.Paddle(HeadOuter, TrackWidth / 2, PaddleTransform(detent)));
        }

        DrawCap(dc, value);
    }

    /// <summary>
    /// The cap: a lit grip with the channel's colour across its middle.
    /// </summary>
    /// <remarks>
    /// <para>
    /// It was a flat rectangle with two grey ridges, and the ridges were the
    /// problem. They were meant to say "take hold of this", but two lines of
    /// the same weight in the middle of a small shape just make it look
    /// textured, and at 100% - where the cap sits right beside a detent that
    /// has gone the channel's colour - the pair read as two unrelated pills.
    /// </para>
    /// <para>
    /// A real fader cap has one index line across it, and that line is where
    /// you read the value from. Using the channel's accent for it does three
    /// things at once: it makes the cap a cap, it puts the value's exact
    /// position on a single pixel row, and it ties the handle to the strip it
    /// belongs to. The top-to-bottom gradient is the rest of it - a flat fill
    /// is a rectangle, a lit one is an object.
    /// </para>
    /// </remarks>
    private void DrawCap(DrawingContext dc, double value)
    {
        var centre = CentreOf(value);
        var rect = new Rect(
            (ActualWidth - CapWidth) / 2,
            centre - CapHeight / 2,
            CapWidth,
            CapHeight);

        dc.DrawRoundedRectangle(Muted ? CapPaint.MutedFace : CapPaint.Face, CapPaint.Edge, rect, 5, 5);

        // Snapped to a whole pixel: a 2px line on a half-pixel boundary is
        // drawn across three rows at reduced contrast, which on a marker this
        // small is the difference between a line and a smudge.
        var index = CapPaint.Index(
            Muted ? (Brush)Application.Current.Resources["MutedGrey"] : Accent);

        var y = Math.Round(centre);
        dc.DrawLine(index, new Point(rect.Left + 6, y), new Point(rect.Right - 6, y));
    }

    /// <summary>The meter's fill and its lit top edge, for the current accent.</summary>
    private void EnsureMeterBrushes()
    {
        if (ReferenceEquals(_meterFor, Accent) && _meterBrush is not null)
        {
            return;
        }

        _meterFor = Accent;

        var colour = Accent is SolidColorBrush solid ? solid.Color : Colors.Gray;

        // Relative to the fill rectangle, so the gradient always runs across
        // whatever is lit rather than across the whole track. A quiet channel
        // gets a small bright wedge instead of the dim top of a long ramp.
        var gradient = new LinearGradientBrush
        {
            StartPoint = new Point(0, 0),
            EndPoint = new Point(0, 1),
        };
        gradient.GradientStops.Add(new GradientStop(Color.FromArgb(0x6E, colour.R, colour.G, colour.B), 0));
        gradient.GradientStops.Add(new GradientStop(Color.FromArgb(0xB4, colour.R, colour.G, colour.B), 1));
        gradient.Freeze();
        _meterBrush = gradient;

        var peak = new SolidColorBrush(colour) { Opacity = 0.95 };
        peak.Freeze();
        _meterPeak = peak;
    }

    // The handle's own brushes live in CapPaint, shared with the Game/Chat
    // mix: the two controls are the same object seen twice and have to look
    // it.

    /// <summary>A detent's fill when the pointer is on it.</summary>
    private static Brush HoverFill(Brush accent)
    {
        var brush = accent.Clone();
        brush.Opacity = 0.55;
        brush.Freeze();
        return brush;
    }

    /// <summary>The value as a percentage, for a readout.</summary>
    public string DisplayText =>
        Muted ? "Muted" : Math.Round(Displayed * 100).ToString(CultureInfo.InvariantCulture) + "%";
}
