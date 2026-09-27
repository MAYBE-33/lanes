using System.Windows;
using Microsoft.Win32;

namespace Lanes.Ui;

/// <summary>
/// Light or dark: which one, and telling the windows when it changes.
/// </summary>
/// <remarks>
/// <para>
/// <b>Which one.</b> The user's choice lives in the core
/// (<c>settings.theme</c>: <c>system</c>, <c>dark</c> or <c>light</c>), because
/// windows here are destroyed when they close and cannot remember anything.
/// <c>system</c> means Windows' own "app mode", read from
/// <c>HKCU\...\Themes\Personalize\AppsUseLightTheme</c> - the same value
/// Explorer and Settings follow - and followed live when it changes.
/// </para>
/// <para>
/// <b>Before the first state arrives</b> the choice is read from the config
/// file, the same way the window already reads its API token from there
/// (<see cref="Api.CoreClient.SavedTheme"/>). Waiting for the connection
/// instead would draw every window dark and then flash it light a moment
/// later. It is only a starting point: the core's answer replaces it as soon
/// as there is one.
/// </para>
/// <para>
/// <b>How a change is applied: the window is rebuilt.</b> Every colour in this
/// window is looked up once, when the control that uses it is made, and
/// controls here are deliberately made once and kept (see
/// <see cref="MainWindow"/>). Making every one of the few hundred lookups
/// dynamic would be a large change with a long tail of missed ones - each
/// missed one a dark patch on a light window. Swapping the palette and
/// building the window again is one mechanism, and cannot miss anything. It
/// happens only when the theme actually changes, which is rare.
/// </para>
/// </remarks>
internal static class Themes
{
    private static readonly Uri LightPalette = new("pack://application:,,,/ThemeLight.xaml", UriKind.Absolute);

    private static string _choice = "system";

    /// <summary>Whether the light palette is the one in use.</summary>
    public static bool Light { get; private set; }

    /// <summary>
    /// The palette has just been swapped. Rebuild whatever is on screen.
    /// </summary>
    /// <remarks>
    /// Raised on the UI thread, and never while a state push is being applied:
    /// rebuilding a window from inside its own state handler would tear down
    /// the object that is still running.
    /// </remarks>
    public static event Action? Changed;

    /// <summary>Apply the saved choice before any window exists.</summary>
    public static void Initialise(string? saved)
    {
        _choice = Normalise(saved);
        Light = Resolve();
        Swap();

        // Windows' own setting, for "system". UserPreferenceChanged arrives for
        // all sorts of reasons; General is the category the app-mode switch
        // raises, and Resolve decides whether anything actually changed.
        SystemEvents.UserPreferenceChanged += (_, e) =>
        {
            if (e.Category == UserPreferenceCategory.General && _choice == "system")
            {
                Application.Current?.Dispatcher.BeginInvoke(Reevaluate);
            }
        };
    }

    /// <summary>What the core says the user chose. Call on every state push.</summary>
    public static void Choose(string? choice)
    {
        var normalised = Normalise(choice);
        if (normalised == _choice)
        {
            return;
        }

        _choice = normalised;
        Application.Current?.Dispatcher.BeginInvoke(Reevaluate);
    }

    /// <summary>The user's choice as the core reported it: system, dark or light.</summary>
    public static string Choice => _choice;

    private static void Reevaluate()
    {
        var light = Resolve();
        if (light == Light)
        {
            return;
        }

        Light = light;
        Swap();
        Controls.CapPaint.Reset();
        Changed?.Invoke();
    }

    private static bool Resolve() => _choice switch
    {
        "light" => true,
        "dark" => false,
        _ => SystemPrefersLight(),
    };

    private static string Normalise(string? choice) => choice switch
    {
        "light" or "dark" => choice,
        _ => "system",
    };

    /// <summary>Windows' app mode. Missing means light: that is Windows' own default.</summary>
    private static bool SystemPrefersLight()
    {
        try
        {
            using var key = Registry.CurrentUser.OpenSubKey(
                @"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
            return key?.GetValue("AppsUseLightTheme") is not int value || value != 0;
        }
        catch
        {
            return false;
        }
    }

    /// <summary>
    /// Put the light palette on top of the dark one, or take it off.
    /// </summary>
    /// <remarks>
    /// Later merged dictionaries win a lookup, so the light file only has to
    /// hold colours; the type scale and the styles stay in Theme.xaml.
    /// </remarks>
    private static void Swap()
    {
        var merged = Application.Current.Resources.MergedDictionaries;
        var existing = merged.FirstOrDefault(d => d.Source?.OriginalString == LightPalette.OriginalString);

        if (Light && existing is null)
        {
            merged.Add(new ResourceDictionary { Source = LightPalette });
        }
        else if (!Light && existing is not null)
        {
            merged.Remove(existing);
        }
    }
}
