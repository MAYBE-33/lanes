using System.Diagnostics;
using System.IO;

namespace Lanes.Ui.Api;

/// <summary>
/// Starting the core when this window cannot find one, and being one app in
/// the taskbar.
/// </summary>
/// <remarks>
/// <para>
/// <b>Why a pure API client is allowed to launch the core.</b> The window holds
/// no authoritative state and cannot reach Windows audio even by accident; that
/// boundary is a process boundary and it is not weakened by knowing the name of
/// the executable sitting next to it. What would weaken it is the window
/// reading a device list or a rule directly, and nothing here does.
/// </para>
/// <para>
/// <b>Why it has to.</b> Lanes ships as two executables — the core with the
/// tray, and this window. Pinning an app to the Windows taskbar pins the
/// executable of the window you are looking at, which is <i>this</i> one. So a
/// perfectly reasonable thing to do leaves a shortcut that starts a mixer with
/// no core behind it: no devices, no channels, nothing working. This file is
/// what prevents that.
/// </para>
/// </remarks>
internal static class CoreProcess
{
    /// <summary>
    /// The identity Windows groups and pins this application under.
    /// </summary>
    /// <remarks>
    /// <para>
    /// Setting this, and setting the same string on the Start Menu shortcut
    /// that points at <c>Lanes.exe</c>, is what makes Windows pin <b>the
    /// shortcut</b> rather than this executable when somebody pins the running
    /// window. It is the documented fix for exactly this shape of app, and it
    /// is why <see cref="Adopt"/> runs before any window exists — Windows reads
    /// it when the first window is created and ignores changes afterwards.
    /// </para>
    /// <para>
    /// The value is arbitrary but must never change once shipped: an existing
    /// pin records it, and a different one silently stops matching.
    /// </para>
    /// </remarks>
    private const string AppId = "MAYBE-33.Lanes";

    /// <summary>
    /// Whether this is a development copy of Lanes running beside an installed
    /// one: a portable copy started with <c>LANES_SEPARATE_INSTANCE=1</c>,
    /// which the core passes on to the windows it starts.
    /// </summary>
    /// <remarks>
    /// A development copy's mixer would otherwise be identical to the real one
    /// and grouped under the same taskbar button - far too easy to use by
    /// mistake. So it has its own taskbar identity and says what it is in every
    /// title. See <c>docs/building.md</c>.
    /// </remarks>
    public static bool IsTestCopy { get; } =
        Environment.GetEnvironmentVariable("LANES_SEPARATE_INSTANCE") == "1"
        && System.IO.File.Exists(System.IO.Path.Combine(AppContext.BaseDirectory, "portable.txt"));

    /// <summary>A window title, marked when this is a test copy.</summary>
    public static string Titled(string title) => IsTestCopy ? $"{title} (test copy)" : title;

    [System.Runtime.InteropServices.DllImport("shell32.dll")]
    private static extern int SetCurrentProcessExplicitAppUserModelID(
        [System.Runtime.InteropServices.MarshalAs(
            System.Runtime.InteropServices.UnmanagedType.LPWStr)] string id);

    /// <summary>Declare which application this process belongs to.</summary>
    public static void Adopt()
    {
        try
        {
            SetCurrentProcessExplicitAppUserModelID(IsTestCopy ? AppId + ".TestCopy" : AppId);
        }
        catch (DllNotFoundException)
        {
            // Cosmetic only: without it the taskbar groups this window under
            // its own executable, which is the behaviour we are improving on,
            // not something to refuse to start over.
        }
        catch (EntryPointNotFoundException)
        {
        }
    }

    /// <summary>Where the core should be: beside this executable.</summary>
    /// <remarks>
    /// The installer puts both in one folder and the build puts both in
    /// <c>dist\</c>, so "beside me" is true in every arrangement Lanes ships
    /// in. Searching the PATH instead would risk starting a different copy.
    /// </remarks>
    public static string? Path
    {
        get
        {
            var here = System.IO.Path.GetDirectoryName(Environment.ProcessPath);
            if (here is null)
            {
                return null;
            }

            var exe = System.IO.Path.Combine(here, "Lanes.exe");
            return File.Exists(exe) ? exe : null;
        }
    }

    /// <summary>
    /// Ask for a core to exist. Returns false if one could not be started.
    /// </summary>
    /// <remarks>
    /// <c>--ensure-core</c> rather than <c>--minimised</c>, and the difference
    /// matters: the minimised path answers "already running" by telling the
    /// core to show its window, so losing a race here would open a second
    /// mixer. <c>--ensure-core</c> starts one or does nothing, and never opens
    /// anything.
    /// </remarks>
    public static bool Start()
    {
        if (Path is not { } exe)
        {
            return false;
        }

        try
        {
            var started = Process.Start(new ProcessStartInfo(exe, "--ensure-core")
            {
                UseShellExecute = false,
                CreateNoWindow = true,
                WorkingDirectory = System.IO.Path.GetDirectoryName(exe)!,
            });

            return started is not null;
        }
        catch (Exception)
        {
            // Any failure here is reported by the caller as "could not start",
            // which is more useful than the exception: the user's next step is
            // the same whatever went wrong.
            return false;
        }
    }
}
