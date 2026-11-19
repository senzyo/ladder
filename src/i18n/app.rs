messages! {
    open => ("打开 Ladder", "Open Ladder", "Открыть Ladder");
    directory => ("打开程序目录", "Open application folder", "Открыть папку приложения");
    exit => ("停止并退出", "Stop cores and exit", "Остановить ядра и выйти");
    window_handle(error: impl std::fmt::Display) => ("读取主窗口句柄失败: {error}", "Cannot read the main window handle: {error}", "Не удалось получить дескриптор главного окна: {error}");
    missing_handle => ("无法获取主窗口的 Windows 句柄", "Cannot get the Windows handle of the main window.", "Не удалось получить дескриптор Windows главного окна.");
    tray_update(error: impl std::fmt::Display) => ("更新托盘菜单失败: {error}", "Cannot update the tray menu: {error}", "Не удалось обновить меню в системном трее: {error}");
    update_cancelled(error: impl std::fmt::Display) => ("{error}。更新已取消, 请重新打开 Ladder 后重试。", "{error}. Update cancelled. Reopen Ladder and try again.", "{error}. Обновление отменено. Откройте Ladder заново и повторите попытку.");
    read_directory(error: impl std::fmt::Display) => ("读取程序目录失败: {error}", "Cannot read the application directory: {error}", "Не удалось прочитать папку приложения: {error}");
    missing_directory => ("无法定位程序目录", "Cannot locate the application directory.", "Не удалось найти папку приложения.");
    window_operation(error: impl std::fmt::Display) => ("窗口操作失败: {error}", "Window operation failed: {error}", "Ошибка операции с окном: {error}");
    start_ui(error: impl std::fmt::Display) => ("无法启动 Ladder 界面: {error}", "Cannot start the Ladder interface: {error}", "Не удалось открыть интерфейс Ladder: {error}");
    exit_dns_warning(message: impl std::fmt::Display) => ("核心已停止, DNS 尚未恢复。下次打开 Ladder 时会再次尝试恢复。\n\n{message}", "The cores have stopped, but DNS has not been restored. Ladder will retry recovery the next time it opens.\n\n{message}", "Ядра остановлены, но DNS ещё не восстановлен. Ladder повторит попытку восстановления при следующем запуске.\n\n{message}");
}
