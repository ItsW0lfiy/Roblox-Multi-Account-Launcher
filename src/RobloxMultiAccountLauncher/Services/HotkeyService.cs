using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Interop;

namespace RobloxMultiAccountLauncher.Services;

public sealed class HotkeyService : IDisposable
{
    private const int WmHotkey = 0x0312;
    private const uint ModAlt = 0x0001;
    private const uint ModControl = 0x0002;
    private HwndSource? _source;
    private nint _handle;
    private bool _registered;
    public event EventHandler<int>? FocusClientRequested;

    public bool Enable(Window window)
    {
        Disable();
        _handle = new WindowInteropHelper(window).Handle;
        _source = HwndSource.FromHwnd(_handle);
        _source?.AddHook(WndProc);
        var one = RegisterHotKey(_handle, 1, ModControl | ModAlt, 0x31);
        var two = RegisterHotKey(_handle, 2, ModControl | ModAlt, 0x32);
        _registered = one && two;
        if (!_registered) Disable();
        return _registered;
    }

    public void Disable()
    {
        if (_handle != nint.Zero)
        {
            UnregisterHotKey(_handle, 1);
            UnregisterHotKey(_handle, 2);
        }
        _source?.RemoveHook(WndProc);
        _source = null;
        _handle = nint.Zero;
        _registered = false;
    }

    private nint WndProc(nint hwnd, int msg, nint wParam, nint lParam, ref bool handled)
    {
        if (msg == WmHotkey)
        {
            handled = true;
            FocusClientRequested?.Invoke(this, wParam.ToInt32());
        }
        return nint.Zero;
    }

    public void Dispose() => Disable();

    [DllImport("user32.dll")]
    private static extern bool RegisterHotKey(nint hWnd, int id, uint modifiers, uint virtualKey);
    [DllImport("user32.dll")]
    private static extern bool UnregisterHotKey(nint hWnd, int id);
}
