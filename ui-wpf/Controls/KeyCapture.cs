using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;

namespace Lanes.Ui.Controls;

/// <summary>
/// Click, then press the keys: records a hotkey the way it will be pressed.
/// </summary>
/// <remarks>
/// <para>
/// Typing "Ctrl+Alt+M" into a text box is how a config file does it, not how a
/// person does. This takes the combination by being pressed, and writes it in
/// the names the core understands (<c>core/src/hotkeys.rs</c>), so what is
/// shown is exactly what is stored.
/// </para>
/// <para>
/// <b>It does not judge the combination.</b> Whether <c>Shift+M</c> is
/// acceptable is the core's rule, and the core says why when it refuses -
/// the settings window shows that reason. A second copy of the rule here would
/// be one more thing to keep in step with it.
/// </para>
/// <para>
/// <b>While it is listening, and only then, it holds a low-level keyboard
/// hook.</b> A combination that some program has registered as a hotkey never
/// reaches a window at all - Windows hands it straight to that program - so
/// without the hook the one case that most needs saying ("that is taken") could
/// not even be entered: the box would sit at "Press the keys…" and nothing
/// would happen. A low-level hook sees the keys before hotkey
/// handling does, so any combination can be recorded, and it swallows the
/// press, so recording Lanes' own live hotkey does not also run it.
/// </para>
/// <para>
/// The hook is installed when the box takes focus and removed when it loses
/// it, and it looks at nothing but the one combination being recorded. A
/// keyboard hook is a powerful thing to hold; this holds it for the few
/// seconds it takes to press two or three keys.
/// </para>
/// </remarks>
public sealed class KeyCapture : Border
{
    private readonly TextBlock _text = new()
    {
        VerticalAlignment = VerticalAlignment.Center,
        TextTrimming = TextTrimming.CharacterEllipsis,
    };

    private string? _keys;
    private bool _listening;

    /// <summary>A complete combination was pressed. Carries it, e.g. <c>Ctrl+Alt+M</c>.</summary>
    public event Action<string>? Captured;

    public KeyCapture()
    {
        Background = Res("Raised");
        BorderBrush = Res("LineStrong");
        BorderThickness = new Thickness(1);
        CornerRadius = new CornerRadius(8);
        Height = 30;
        Padding = new Thickness(10, 0, 10, 0);
        Cursor = Cursors.Hand;
        Focusable = true;
        FocusVisualStyle = null;
        Child = _text;
        ToolTip = "Click, then press the keys";

        MouseLeftButtonUp += (_, _) =>
        {
            Focus();
            Keyboard.Focus(this);
        };

        GotKeyboardFocus += (_, _) =>
        {
            Listen(true);
            Hook(true);
        };
        LostKeyboardFocus += (_, _) =>
        {
            Hook(false);
            Listen(false);
        };
        Unloaded += (_, _) => Hook(false);

        PreviewKeyDown += OnKeyDown;

        Show();
    }

    /// <summary>The combination shown, or null for none yet.</summary>
    public string? Keys
    {
        get => _keys;
        set
        {
            _keys = value;
            Show();
        }
    }

    private void Listen(bool on)
    {
        _listening = on;
        BorderBrush = Res(on ? "TextFaint" : "LineStrong");
        Show();
    }

    private void Show()
    {
        if (_listening)
        {
            _text.Text = "Press the keys…";
            _text.Foreground = Res("TextFaint");
        }
        else
        {
            _text.Text = _keys ?? "Set keys";
            _text.Foreground = Res(_keys is null ? "TextGhost" : "Text");
        }
    }

    // --- The low-level hook -------------------------------------------------

    private const int WH_KEYBOARD_LL = 13;
    private const int WM_KEYDOWN = 0x0100;
    private const int WM_SYSKEYDOWN = 0x0104;

    [StructLayout(LayoutKind.Sequential)]
    private struct KBDLLHOOKSTRUCT
    {
        public uint vkCode;
        public uint scanCode;
        public uint flags;
        public uint time;
        public IntPtr dwExtraInfo;
    }

    private delegate IntPtr HookProc(int code, IntPtr wParam, IntPtr lParam);

    [DllImport("user32.dll", SetLastError = true)]
    private static extern IntPtr SetWindowsHookEx(int id, HookProc proc, IntPtr module, uint thread);

    [DllImport("user32.dll")]
    private static extern bool UnhookWindowsHookEx(IntPtr hook);

    [DllImport("user32.dll")]
    private static extern IntPtr CallNextHookEx(IntPtr hook, int code, IntPtr wParam, IntPtr lParam);

    [DllImport("user32.dll")]
    private static extern short GetAsyncKeyState(int key);

    [DllImport("kernel32.dll")]
    private static extern IntPtr GetModuleHandle(string? name);

    private IntPtr _hook;

    /// <summary>Held so the garbage collector cannot free what Windows calls back into.</summary>
    private HookProc? _proc;

    private void Hook(bool on)
    {
        if (on && _hook == IntPtr.Zero)
        {
            _proc = OnLowLevelKey;
            _hook = SetWindowsHookEx(WH_KEYBOARD_LL, _proc, GetModuleHandle(null), 0);
        }
        else if (!on && _hook != IntPtr.Zero)
        {
            UnhookWindowsHookEx(_hook);
            _hook = IntPtr.Zero;
            _proc = null;
        }
    }

    private IntPtr OnLowLevelKey(int code, IntPtr wParam, IntPtr lParam)
    {
        var message = (int)wParam;
        if (code < 0 || (message != WM_KEYDOWN && message != WM_SYSKEYDOWN))
        {
            return CallNextHookEx(_hook, code, wParam, lParam);
        }

        var info = Marshal.PtrToStructure<KBDLLHOOKSTRUCT>(lParam);
        var key = KeyInterop.KeyFromVirtualKey((int)info.vkCode);

        // Modifiers go through: they are the start of a combination, and the
        // rest of Windows needs to see them released properly.
        if (IsModifier(key))
        {
            return CallNextHookEx(_hook, code, wParam, lParam);
        }

        // Read here, not later on the dispatcher: by then the keys may be up.
        static bool Down(int vk) => (GetAsyncKeyState(vk) & 0x8000) != 0;
        var ctrl = Down(0x11);
        var alt = Down(0x12);
        var shift = Down(0x10);
        var win = Down(0x5B) || Down(0x5C);

        Dispatcher.BeginInvoke(() => Record(key, ctrl, alt, shift, win));

        // Swallowed: nothing else - neither the program that owns these keys as
        // a hotkey, nor this window - acts on the press being recorded.
        return (IntPtr)1;
    }

    private static bool IsModifier(Key key) =>
        key is Key.LeftCtrl or Key.RightCtrl or Key.LeftAlt or Key.RightAlt
            or Key.LeftShift or Key.RightShift or Key.LWin or Key.RWin;

    private void Record(Key key, bool ctrl, bool alt, bool shift, bool win)
    {
        if (!_listening)
        {
            return;
        }

        if (key == Key.Escape && !ctrl && !alt && !shift && !win)
        {
            Keyboard.ClearFocus();
            return;
        }

        if (KeyName(key) is not { } name)
        {
            _text.Text = $"{key} can't be used";
            return;
        }

        var parts = new List<string>();
        if (ctrl) parts.Add("Ctrl");
        if (alt) parts.Add("Alt");
        if (shift) parts.Add("Shift");
        if (win) parts.Add("Win");
        parts.Add(name);

        _keys = string.Join("+", parts);
        Keyboard.ClearFocus();
        Captured?.Invoke(_keys);
    }

    // --- Without the hook ----------------------------------------------------

    /// <summary>
    /// The ordinary route, for when the hook could not be installed. Keys the
    /// hook has already swallowed never reach it.
    /// </summary>
    private void OnKeyDown(object sender, KeyEventArgs e)
    {
        if (_hook != IntPtr.Zero)
        {
            // The hook is recording; only stop the key moving focus or typing.
            e.Handled = true;
            return;
        }

        // Everything is ours while listening - Tab included, or it would move
        // focus instead of being refused as a key.
        e.Handled = true;

        // Alt combinations arrive as Key.System with the real key alongside.
        var key = e.Key == Key.System ? e.SystemKey : e.Key;

        if (key == Key.Escape)
        {
            Keyboard.ClearFocus();
            return;
        }

        // A modifier on its own is the start of a combination, not the end.
        if (key is Key.LeftCtrl or Key.RightCtrl or Key.LeftAlt or Key.RightAlt
            or Key.LeftShift or Key.RightShift or Key.LWin or Key.RWin)
        {
            return;
        }

        if (KeyName(key) is not { } name)
        {
            _text.Text = $"{key} can't be used";
            return;
        }

        var parts = new List<string>();
        var modifiers = Keyboard.Modifiers;
        if (modifiers.HasFlag(ModifierKeys.Control)) parts.Add("Ctrl");
        if (modifiers.HasFlag(ModifierKeys.Alt)) parts.Add("Alt");
        if (modifiers.HasFlag(ModifierKeys.Shift)) parts.Add("Shift");
        if (modifiers.HasFlag(ModifierKeys.Windows)) parts.Add("Win");
        parts.Add(name);

        _keys = string.Join("+", parts);
        Keyboard.ClearFocus();
        Captured?.Invoke(_keys);
    }

    /// <summary>A WPF key in the name the core's parser uses, or null for one it does not.</summary>
    private static string? KeyName(Key key)
    {
        if (key is >= Key.A and <= Key.Z)
        {
            return key.ToString();
        }
        if (key is >= Key.D0 and <= Key.D9)
        {
            return ((int)(key - Key.D0)).ToString();
        }
        if (key is >= Key.F1 and <= Key.F24)
        {
            return key.ToString();
        }
        if (key is >= Key.NumPad0 and <= Key.NumPad9)
        {
            return key.ToString();
        }

        return key switch
        {
            Key.Space => "Space",
            Key.PageUp => "PageUp",
            Key.PageDown => "PageDown",
            Key.End => "End",
            Key.Home => "Home",
            Key.Left => "Left",
            Key.Up => "Up",
            Key.Right => "Right",
            Key.Down => "Down",
            Key.Insert => "Insert",
            Key.Delete => "Delete",
            Key.Pause => "Pause",
            Key.Scroll => "ScrollLock",
            Key.OemPlus => "Plus",
            Key.OemComma => "Comma",
            Key.OemMinus => "Minus",
            Key.OemPeriod => "Period",
            Key.VolumeMute => "VolumeMute",
            Key.VolumeDown => "VolumeDown",
            Key.VolumeUp => "VolumeUp",
            Key.MediaNextTrack => "MediaNext",
            Key.MediaPreviousTrack => "MediaPrevious",
            Key.MediaStop => "MediaStop",
            Key.MediaPlayPause => "MediaPlayPause",
            _ => null,
        };
    }

    private static Brush Res(string key) => (Brush)Application.Current.Resources[key];
}
