messages! {
    latency_config => (
        "无法准备此配置的测速副本", "Cannot prepare a copy of this configuration for the latency test",
        "Не удалось подготовить копию этой конфигурации для проверки задержки"
    );
    latency_cache(error: impl std::fmt::Display) => (
        "无法准备 mihomo 的测速缓存副本: {error}", "Could not prepare a copy of the mihomo cache for the latency test: {error}",
        "Не удалось подготовить копию кеша mihomo для проверки задержки: {error}"
    );
    latency_endpoint => (
        "此配置使用的网络端点暂不支持隔离测速", "Isolated latency tests are not yet supported for the network endpoint used by this configuration",
        "Для сетевой конечной точки этой конфигурации пока не поддерживается изолированная проверка задержки"
    );
    latency_wireguard_conflict => (
        "另一份正在运行或测速的配置使用了同一 WireGuard 身份, 无法为此配置单独测速。",
        "Another active configuration or latency test uses the same WireGuard identity. This configuration cannot be tested separately.",
        "Другая активная конфигурация или проверка использует тот же идентификатор WireGuard. Отдельная проверка недоступна."
    );
    latency_wireguard_unknown => (
        "无法确定所有 WireGuard 身份。为避免影响现有连接, 请在相关核心或测速停止后重试。",
        "The WireGuard identities could not all be determined. Retry after the other core or latency test stops to avoid interrupting its connection.",
        "Не удалось определить все идентификаторы WireGuard. Чтобы не прервать соединение, повторите после остановки другого ядра или проверки."
    );
    latency_wireguard_no_proxy => (
        "运行中的配置没有可复用的本地 HTTP 或 SOCKS5 代理入口, 无法安全地测试这条线路。",
        "The running configuration has no usable local HTTP or SOCKS5 proxy listener for a safe latency test.",
        "В работающей конфигурации нет доступного локального прокси HTTP или SOCKS5 для безопасной проверки задержки."
    );
    latency_start => (
        "测速核心启动失败, 请检查配置和所选版本", "The test core could not start. Check the configuration and selected version.",
        "Не удалось запустить ядро для проверки. Проверьте конфигурацию и выбранную версию."
    );
    latency_start_timeout => (
        "等待测速核心就绪超时", "Timed out waiting for the test core to become ready",
        "Истекло время ожидания готовности ядра для проверки задержки"
    );
    latency_cancelled => (
        "测速已取消", "Latency test cancelled", "Проверка задержки отменена"
    );
    latency_timeout => (
        "测速请求超时", "Latency request timed out", "Истекло время ожидания ответа при проверке задержки"
    );
    latency_request => (
        "无法通过此配置访问测速 URL", "Could not reach the test URL through this configuration",
        "Не удалось открыть тестовый URL через эту конфигурацию"
    );
    latency_status(status: u16) => (
        "测速地址返回 HTTP {status}", "The test URL returned HTTP {status}",
        "Тестовый URL вернул HTTP {status}"
    );
    dns_read_record(path: impl std::fmt::Display, error: impl std::fmt::Display) => (
        "无法读取 DNS 恢复记录 {path}: {error}",
        "Cannot read DNS recovery record {path}: {error}",
        "Не удалось прочитать запись восстановления DNS {path}: {error}"
    );
    dns_invalid_record(path: impl std::fmt::Display, line: usize, column: usize) => (
        "{path} 格式无效 (第 {line} 行, 第 {column} 列) 。请用备份修复记录, 或在手动恢复网卡 DNS 后移走此文件。",
        "{path} has an invalid format (line {line}, column {column}). Restore the record from a backup, or move this file away after manually restoring adapter DNS.",
        "Неверный формат {path} (строка {line}, столбец {column}). Восстановите запись из резервной копии или переместите этот файл после ручного восстановления DNS адаптеров."
    );
    dns_save_record(path: impl std::fmt::Display, error: impl std::fmt::Display) => (
        "无法保存 DNS 恢复记录 {path}: {error}",
        "Cannot save DNS recovery record {path}: {error}",
        "Не удалось сохранить запись восстановления DNS {path}: {error}"
    );
    dns_remove_old_record(path: impl std::fmt::Display, error: impl std::fmt::Display) => (
        "无法清除旧 DNS 恢复记录 {path}: {error}",
        "Cannot remove old DNS recovery record {path}: {error}",
        "Не удалось удалить старую запись восстановления DNS {path}: {error}"
    );
    dns_restore_failed(error: impl std::fmt::Display) => (
        "恢复 DNS 失败: {error}",
        "Failed to restore DNS: {error}",
        "Не удалось восстановить DNS: {error}"
    );
    dns_remove_record(path: impl std::fmt::Display, error: impl std::fmt::Display) => (
        "无法清除 DNS 恢复记录 {path}: {error}",
        "Cannot remove DNS recovery record {path}: {error}",
        "Не удалось удалить запись восстановления DNS {path}: {error}"
    );
    dns_flush_failed => (
        "刷新 DNS 客户端缓存失败",
        "Failed to flush the DNS client cache",
        "Не удалось очистить кеш DNS-клиента"
    );
    dns_check_record_directory(path: impl std::fmt::Display, error: impl std::fmt::Display) => (
        "无法核对 DNS 恢复记录 {path} 的目录: {error}",
        "Cannot verify the directory of DNS recovery record {path}: {error}",
        "Не удалось проверить каталог записи восстановления DNS {path}: {error}"
    );
    dns_check_old_record(path: impl std::fmt::Display, error: impl std::fmt::Display) => (
        "无法检查旧 DNS 恢复记录 {path}: {error}",
        "Cannot inspect old DNS recovery record {path}: {error}",
        "Не удалось проверить старую запись восстановления DNS {path}: {error}"
    );
    dns_open_old_lock(path: impl std::fmt::Display, error: impl std::fmt::Display) => (
        "无法打开旧 DNS 恢复锁 {path}: {error}",
        "Cannot open old DNS recovery lock {path}: {error}",
        "Не удалось открыть старую блокировку восстановления DNS {path}: {error}"
    );
    dns_acquire_old_lock(path: impl std::fmt::Display, error: impl std::fmt::Display) => (
        "无法取得旧 DNS 恢复锁 {path}: {error}",
        "Cannot acquire old DNS recovery lock {path}: {error}",
        "Не удалось получить старую блокировку восстановления DNS {path}: {error}"
    );
    dns_enumerate_adapters(error: impl std::fmt::Display) => (
        "无法枚举物理网卡: {error}",
        "Cannot list physical network adapters: {error}",
        "Не удалось получить список физических сетевых адаптеров: {error}"
    );
    dns_query_protocol(error: impl std::fmt::Display) => (
        "无法查询网卡协议: {error}",
        "Cannot query adapter protocols: {error}",
        "Не удалось запросить протоколы адаптера: {error}"
    );
    dns_read_adapter(error: impl std::fmt::Display) => (
        "无法读取网卡, 请检查网卡是否可用: {error}",
        "Cannot read the network adapter. Check that it is available: {error}",
        "Не удалось прочитать данные сетевого адаптера. Проверьте его доступность: {error}"
    );
    dns_read_settings(error: impl std::fmt::Display) => (
        "无法读取网卡 DNS 设置: {error}",
        "Cannot read adapter DNS settings: {error}",
        "Не удалось прочитать настройки DNS адаптера: {error}"
    );
    dns_access_denied => (
        "Windows 拒绝设置网卡 DNS, 请检查网卡权限或组织策略",
        "Windows denied the adapter DNS change. Check adapter permissions or organization policies",
        "Windows запретила изменение DNS адаптера. Проверьте права доступа к адаптеру и политики организации"
    );
    dns_write_settings(error: impl std::fmt::Display) => (
        "设置网卡 DNS 失败: {error}",
        "Failed to set adapter DNS: {error}",
        "Не удалось настроить DNS адаптера: {error}"
    );
    dns_open_restore_settings(error: impl std::fmt::Display) => (
        "无法打开待恢复的网卡 DNS 设置: {error}",
        "Cannot open adapter DNS settings for restoration: {error}",
        "Не удалось открыть настройки DNS адаптера для восстановления: {error}"
    );
    dns_restore_offline(error: impl std::fmt::Display) => (
        "无法恢复离线网卡 DNS: {error}",
        "Cannot restore DNS for an offline adapter: {error}",
        "Не удалось восстановить DNS отключённого адаптера: {error}"
    );
    error_separator => (
        "；",
        "; ",
        "; "
    );
    combined_errors(first: impl std::fmt::Display, second: impl std::fmt::Display) => (
        "{first}；{second}",
        "{first}; {second}",
        "{first}; {second}"
    );
    system_folder_failed(status: i32) => (
        "无法读取 Windows 系统目录 (HRESULT {status:#x})",
        "Cannot read the Windows system directory (HRESULT {status:#x})",
        "Не удалось определить системный каталог Windows (HRESULT {status:#x})"
    );
    recovery_directory_verify => (
        "无法验证机器级 DNS 恢复目录",
        "Cannot verify the system-wide DNS recovery directory",
        "Не удалось проверить общесистемный каталог восстановления DNS"
    );
    directory_reparse_point => (
        "目录是重解析点",
        "The directory is a reparse point",
        "Каталог является точкой повторного анализа"
    );
    directory_permissions => (
        "目录权限不符合 SYSTEM 任务要求",
        "Directory permissions do not meet the requirements for a SYSTEM task",
        "Права доступа к каталогу не соответствуют требованиям задачи SYSTEM"
    );
    directory_unexpected_contents => (
        "恢复目录包含非预期文件、子目录或重解析点",
        "The recovery directory contains unexpected files, subdirectories, or reparse points",
        "Каталог восстановления содержит непредусмотренные файлы, подкаталоги или точки повторного анализа"
    );
    recovery_file_permissions => (
        "恢复文件权限不符合 SYSTEM 任务要求",
        "Recovery file permissions do not meet the requirements for a SYSTEM task",
        "Права доступа к файлам восстановления не соответствуют требованиям задачи SYSTEM"
    );
    recovery_directory_check(path: impl std::fmt::Display) => (
        "无法创建或检查 {path}, 请检查目录权限和占用情况",
        "Cannot create or inspect {path}. Check directory permissions and whether it is in use",
        "Не удалось создать или проверить {path}. Проверьте права доступа и убедитесь, что каталог не занят"
    );
    recovery_check_interrupted(path: impl std::fmt::Display) => (
        "检查 {path} 的助手异常结束, 请重试",
        "The helper checking {path} exited unexpectedly. Please retry",
        "Помощник проверки {path} завершился неожиданно. Повторите попытку"
    );
    dns_acquire_lock(error: impl std::fmt::Display) => (
        "无法取得 DNS 恢复锁: {error}",
        "Cannot acquire the DNS recovery lock: {error}",
        "Не удалось получить блокировку восстановления DNS: {error}"
    );
    dns_open_portable_lock => (
        "无法打开便携 DNS 恢复锁",
        "Cannot open the portable DNS recovery lock",
        "Не удалось открыть блокировку переносимой записи восстановления DNS"
    );
    dns_machine_unavailable(error: impl std::fmt::Display) => (
        "无法安全访问机器级 DNS 恢复记录。请检查恢复目录, 或等待占用结束后重试: {error:#}",
        "Cannot safely access system-wide DNS recovery records. Check the recovery directory, or retry when it is no longer in use: {error:#}",
        "Не удалось безопасно получить доступ к общесистемным записям восстановления DNS. Проверьте каталог восстановления или повторите попытку, когда он освободится: {error:#}"
    );
    dns_portable_warning(error: impl std::fmt::Display) => (
        "本次使用便携 DNS 恢复记录, 只能由 Ladder 恢复。{error:#}",
        "This session uses portable DNS recovery records, which only Ladder can restore. {error:#}",
        "В этом сеансе используются переносимые записи восстановления DNS. Восстановить их может только Ladder. {error:#}"
    );
    dns_open_lock => (
        "无法打开 DNS 恢复锁",
        "Cannot open the DNS recovery lock",
        "Не удалось открыть блокировку восстановления DNS"
    );
    dns_boot_task_update(error: impl std::fmt::Display) => (
        "更新开机 DNS 恢复任务失败: {error:#}",
        "Failed to update the startup task for DNS recovery: {error:#}",
        "Не удалось обновить задачу для восстановления DNS при загрузке системы: {error:#}"
    );
    recovery_helper_write => (
        "写入开机 DNS 恢复程序失败",
        "Failed to write the DNS recovery program for startup",
        "Не удалось записать программу восстановления DNS при загрузке"
    );
    recovery_helper_update => (
        "更新开机 DNS 恢复程序失败",
        "Failed to update the DNS recovery program for startup",
        "Не удалось обновить программу восстановления DNS при загрузке"
    );
    task_scheduler_start => (
        "无法启动 Windows 任务计划工具",
        "Cannot start the Windows Task Scheduler tool",
        "Не удалось запустить средство планировщика заданий Windows"
    );
    dns_task_register => (
        "无法注册开机 DNS 恢复任务, 请检查管理员权限和 Windows 任务计划程序服务",
        "Cannot register the startup task for DNS recovery. Check administrator permissions and the Windows Task Scheduler service",
        "Не удалось зарегистрировать задачу для восстановления DNS при загрузке системы. Проверьте права администратора и службу планировщика заданий Windows"
    );
    core_config_read(error: impl std::fmt::Display) => (
        "读取核心配置失败: {error}",
        "Failed to read the core configuration: {error}",
        "Не удалось прочитать конфигурацию ядра: {error}"
    );
    core_config_invalid(line: usize, column: usize) => (
        "核心配置格式无效 (第 {line} 行, 第 {column} 列)",
        "Invalid core configuration format (line {line}, column {column})",
        "Неверный формат конфигурации ядра (строка {line}, столбец {column})"
    );
    xray_tun_settings_object => (
        "Xray 的 TUN settings 必须是对象",
        "Xray TUN settings must be an object",
        "Параметр settings для TUN в Xray должен быть объектом"
    );
    tun_field_string(key: impl std::fmt::Display) => (
        "TUN 的 {key} 必须是字符串",
        "TUN {key} must be a string",
        "Параметр {key} для TUN должен быть строкой"
    );
    core_config_directory => (
        "无法定位核心配置目录",
        "Cannot locate the core configuration directory",
        "Не удалось найти каталог конфигурации ядра"
    );
    tun_config_create(error: impl std::fmt::Display) => (
        "创建 TUN 运行配置失败: {error}",
        "Failed to create the TUN runtime configuration: {error}",
        "Не удалось создать рабочую конфигурацию TUN: {error}"
    );
    tun_config_write => (
        "写入 TUN 运行配置失败",
        "Failed to write the TUN runtime configuration",
        "Не удалось записать рабочую конфигурацию TUN"
    );
    tun_config_write_error(error: impl std::fmt::Display) => (
        "写入 TUN 运行配置失败: {error}",
        "Failed to write the TUN runtime configuration: {error}",
        "Не удалось записать рабочую конфигурацию TUN: {error}"
    );
    core_yaml_invalid => (
        "核心配置 YAML 格式无效",
        "Invalid YAML core configuration",
        "Неверный формат YAML конфигурации ядра"
    );
    core_yaml_invalid_at(line: usize, column: usize) => (
        "核心配置 YAML 格式无效 (第 {line} 行, 第 {column} 列)",
        "Invalid YAML core configuration (line {line}, column {column})",
        "Неверный формат YAML конфигурации ядра (строка {line}, столбец {column})"
    );
    core_yaml_merge_invalid => (
        "核心配置 YAML 合并字段无效",
        "Invalid YAML merge field in the core configuration",
        "Недопустимое поле слияния YAML в конфигурации ядра"
    );
    core_directory_missing => (
        "无法定位核心目录",
        "Cannot locate the core directory",
        "Не удалось найти каталог ядра"
    );
    managed_process_stop_timeout => (
        "等待托管进程退出超时, 请重试停止",
        "Timed out waiting for the managed process to exit. Try stopping it again",
        "Истекло время ожидания завершения управляемого процесса. Повторите попытку остановки"
    );
    core_output_thread_failed => (
        "核心输出读取线程异常退出",
        "The core output reader thread exited unexpectedly",
        "Поток чтения вывода ядра завершился неожиданно"
    );
    core_output_read_failed => (
        "读取核心输出失败, 诊断可能不完整",
        "Failed to read core output; diagnostics may be incomplete",
        "Не удалось прочитать вывод ядра; диагностические данные могут быть неполными"
    );
    parent_handle_missing => (
        "缺少有效的父进程句柄",
        "A valid parent process handle is missing",
        "Отсутствует действительный дескриптор родительского процесса"
    );
    parent_wait_failed => (
        "等待异常进程退出失败",
        "Failed to wait for the crashed process to exit",
        "Не удалось дождаться завершения аварийного процесса"
    );
    dns_recovery_directory_missing => (
        "缺少 DNS 恢复目录",
        "The DNS recovery directory is missing",
        "Не указан каталог восстановления DNS"
    );
    dns_recovery_arguments => (
        "DNS 恢复参数过多",
        "Too many DNS recovery arguments",
        "Слишком много аргументов восстановления DNS"
    );
    dns_recovery_absolute_path => (
        "DNS 恢复目录必须是绝对路径",
        "The DNS recovery directory must be an absolute path",
        "Путь к каталогу восстановления DNS должен быть абсолютным"
    );
    recovery_argument_nul => (
        "恢复助手参数包含 NUL",
        "A recovery helper argument contains NUL",
        "Аргумент помощника восстановления содержит NUL"
    );
    recovery_command_too_long => (
        "恢复助手命令行过长",
        "The recovery helper command line is too long",
        "Слишком длинная командная строка помощника восстановления"
    );
}

#[cfg(not(test))]
messages! {
    network_interface_table_missing => (
        "系统未返回网络接口表",
        "The system did not return a network interface table",
        "Система не вернула таблицу сетевых интерфейсов"
    );
    tun_cleanup_failed => (
        "无法清理残留 TUN 设备, 请检查管理员权限及设备是否被占用",
        "Cannot remove leftover TUN devices. Check administrator permissions and whether the devices are in use",
        "Не удалось удалить оставшиеся устройства TUN. Проверьте права администратора и убедитесь, что устройства не заняты"
    );
    pnputil_timeout => (
        "等待 pnputil 超时",
        "Timed out waiting for pnputil",
        "Истекло время ожидания pnputil"
    );
}
