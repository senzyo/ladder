messages! {
    invalid_proxy_prefix => (
        "GitHub 加速链接前缀须为 HTTPS 地址, 不能包含账号、密码、查询参数或片段",
        "The GitHub proxy prefix must be an HTTPS URL without credentials, query parameters or fragments",
        "Префикс прокси GitHub должен быть HTTPS-адресом без учётных данных, параметров запроса и фрагментов"
    );
    invalid_github_url => (
        "GitHub 资源地址无效",
        "Invalid GitHub resource URL",
        "Недопустимый адрес ресурса GitHub"
    );
    not_a_directory => (
        "所选路径不是目录",
        "The selected path is not a directory",
        "Выбранный путь не является каталогом"
    );
    not_a_config_file => (
        "所选路径不是配置文件",
        "The selected path is not a configuration file",
        "Выбранный путь не является файлом конфигурации"
    );
    unsupported_config_type => (
        "只能编辑 JSON 或 YAML 配置文件",
        "Only JSON or YAML configuration files can be edited",
        "Редактировать можно только файлы конфигурации JSON или YAML"
    );
    notepad_path_too_long => (
        "无法定位系统记事本: 系统目录路径过长",
        "Cannot locate Notepad: the system directory path is too long",
        "Не удалось найти Блокнот: путь к системному каталогу слишком длинный"
    );
    invalid_app_release => (
        "GitHub 返回了无效的软件发布信息",
        "GitHub returned invalid application release information",
        "GitHub вернул некорректные сведения о выпуске приложения"
    );
    invalid_app_version => (
        "软件发布版本号无效",
        "Invalid application release version",
        "Недопустимый номер версии приложения"
    );
    app_stable_only => (
        "软件更新只接受正式发布版本",
        "Application updates only support stable releases",
        "Обновление приложения поддерживает только стабильные выпуски"
    );
    cannot_compare_app_versions => (
        "无法比较软件版本号",
        "Cannot compare application versions",
        "Не удалось сравнить версии приложения"
    );
    missing_app_architecture => (
        "该版本没有适用于当前架构的软件安装包",
        "This release has no application package for the current architecture",
        "В этом выпуске нет пакета приложения для текущей архитектуры"
    );
    invalid_app_package_info => (
        "软件安装包发布信息无效",
        "Invalid application package release information",
        "Некорректные сведения о пакете приложения"
    );
    missing_app_digest => (
        "软件安装包缺少 GitHub SHA-256 校验信息",
        "The application package has no GitHub SHA-256 checksum",
        "Для пакета приложения отсутствует контрольная сумма SHA-256 от GitHub"
    );
    invalid_app_digest => (
        "软件安装包 SHA-256 校验信息无效",
        "Invalid SHA-256 checksum for the application package",
        "Недопустимая контрольная сумма SHA-256 пакета приложения"
    );
    no_app_release => (
        "尚无可用的 Ladder 正式发行版",
        "No stable Ladder release is available yet",
        "Стабильный выпуск Ladder пока недоступен"
    );
    app_request_limited => (
        "GitHub 请求受限, 请稍后重试或更换加速链接前缀",
        "GitHub requests are rate-limited. Try again later or change the proxy prefix",
        "GitHub ограничил частоту запросов. Повторите позже или измените префикс прокси"
    );
    app_package_gone => (
        "软件安装包已不存在, 请重新检查更新",
        "The application package is no longer available. Check for updates again",
        "Пакет приложения больше недоступен. Проверьте обновления ещё раз"
    );
    app_request_failed => (
        "软件更新请求失败或超时, 请检查网络后重试",
        "The application update request failed or timed out. Check your connection and try again",
        "Запрос обновления приложения завершился ошибкой или превысил время ожидания. Проверьте соединение и повторите попытку"
    );
    cannot_locate_app => (
        "无法定位软件程序",
        "Cannot locate the application executable",
        "Не удалось найти исполняемый файл приложения"
    );
    cannot_locate_app_directory => (
        "无法定位程序目录",
        "Cannot locate the application directory",
        "Не удалось найти каталог приложения"
    );
    cannot_create_app_update_directory => (
        "无法创建软件更新目录, 请检查程序目录权限",
        "Cannot create the update directory. Check the application directory permissions",
        "Не удалось создать каталог обновления. Проверьте права доступа к каталогу приложения"
    );
    cannot_create_app_package => (
        "无法保存软件安装包",
        "Cannot save the application package",
        "Не удалось сохранить пакет приложения"
    );
    app_download_interrupted => (
        "软件安装包下载中断或超时",
        "The application package download was interrupted or timed out",
        "Загрузка пакета приложения прервана или превышено время ожидания"
    );
    app_size_mismatch => (
        "软件安装包大小与发布信息不符",
        "The application package size does not match the release information",
        "Размер пакета приложения не соответствует сведениям о выпуске"
    );
    cannot_save_app_package => (
        "保存软件安装包失败",
        "Failed to save the application package",
        "Не удалось сохранить пакет приложения"
    );
    app_verification_failed => (
        "软件安装包大小或 SHA-256 校验失败, 当前软件未修改",
        "The application package size or SHA-256 checksum is incorrect. The current application is unchanged",
        "Размер или контрольная сумма SHA-256 пакета приложения неверны. Текущее приложение не изменено"
    );
    cannot_read_app_package => (
        "无法读取软件安装包",
        "Cannot read the application package",
        "Не удалось прочитать пакет приложения"
    );
    invalid_app_zip => (
        "软件安装包不是有效的 ZIP",
        "The application package is not a valid ZIP archive",
        "Пакет приложения не является корректным ZIP-архивом"
    );
    corrupt_app_package => (
        "软件安装包损坏",
        "The application package is corrupted",
        "Пакет приложения повреждён"
    );
    invalid_app_filename => (
        "软件安装包文件名无效",
        "Invalid file name in the application package",
        "Недопустимое имя файла в пакете приложения"
    );
    app_contents_too_large => (
        "软件安装包内容过大",
        "The application package contents are too large",
        "Содержимое пакета приложения слишком велико"
    );
    unsupported_app_contents => (
        "软件安装包含不支持的文件或内容过大",
        "The application package contains unsupported files or is too large",
        "Пакет приложения содержит неподдерживаемые файлы или слишком велик"
    );
    invalid_app_executable => (
        "软件安装包的程序文件无效",
        "The executable in the application package is invalid",
        "Исполняемый файл в пакете приложения некорректен"
    );
    cannot_stage_app => (
        "无法暂存新版程序",
        "Cannot stage the new application executable",
        "Не удалось подготовить новый исполняемый файл приложения"
    );
    cannot_extract_app => (
        "解压软件安装包失败",
        "Failed to extract the application package",
        "Не удалось распаковать пакет приложения"
    );
    invalid_new_app_size => (
        "新版程序大小无效",
        "Invalid size of the new application executable",
        "Недопустимый размер нового исполняемого файла приложения"
    );
    cannot_save_new_app => (
        "保存新版程序失败",
        "Failed to save the new application executable",
        "Не удалось сохранить новый исполняемый файл приложения"
    );
    missing_ladder_exe => (
        "软件安装包缺少 ladder.exe",
        "The application package does not contain ladder.exe",
        "В пакете приложения отсутствует ladder.exe"
    );
    invalid_new_app_architecture => (
        "新版程序不是适用于当前架构的 Windows 程序",
        "The new executable is not a Windows application for the current architecture",
        "Новый исполняемый файл не является приложением Windows для текущей архитектуры"
    );
    cannot_read_new_app => (
        "无法读取新版程序",
        "Cannot read the new application executable",
        "Не удалось прочитать новый исполняемый файл приложения"
    );
    cannot_verify_new_app => (
        "无法校验新版程序",
        "Cannot verify the new application executable",
        "Не удалось проверить новый исполняемый файл приложения"
    );
    cannot_check_update_helper => (
        "无法检查软件更新助手",
        "Cannot check the application update helper",
        "Не удалось проверить состояние помощника обновления"
    );
    update_helper_exited => (
        "软件更新助手已退出, 当前软件未修改",
        "The update helper has exited. The current application is unchanged",
        "Помощник обновления завершил работу. Текущее приложение не изменено"
    );
    cannot_commit_app_update => (
        "无法确认软件更新, 当前软件未修改",
        "Cannot confirm the application update. The current application is unchanged",
        "Не удалось подтвердить обновление приложения. Текущее приложение не изменено"
    );
    invalid_executable_name => (
        "程序文件名无效",
        "Invalid executable file name",
        "Недопустимое имя исполняемого файла"
    );
    cannot_create_update_plan => (
        "无法生成软件更新计划",
        "Cannot generate the application update plan",
        "Не удалось создать план обновления приложения"
    );
    cannot_save_update_plan => (
        "无法保存软件更新计划",
        "Cannot save the application update plan",
        "Не удалось сохранить план обновления приложения"
    );
    cannot_prepare_update_helper => (
        "无法准备软件更新助手",
        "Cannot prepare the application update helper",
        "Не удалось подготовить помощник обновления приложения"
    );
    cannot_locate_windows => (
        "无法定位 Windows 系统目录",
        "Cannot locate the Windows system directory",
        "Не удалось найти системный каталог Windows"
    );
    cannot_start_update_helper => (
        "无法启动软件更新助手, 当前软件未修改",
        "Cannot start the update helper. The current application is unchanged",
        "Не удалось запустить помощник обновления. Текущее приложение не изменено"
    );
    update_helper_start_failed => (
        "软件更新助手启动失败, 当前软件未修改",
        "The update helper failed to start. The current application is unchanged",
        "Ошибка запуска помощника обновления. Текущее приложение не изменено"
    );
    update_helper_timeout => (
        "软件更新助手未能及时就绪, 当前软件未修改",
        "The update helper did not become ready in time. The current application is unchanged",
        "Помощник обновления не подготовился вовремя. Текущее приложение не изменено"
    );
    previous_update_incomplete => (
        "上次软件更新未完成, 配置已保留。请重试更新, 或退出后手动替换 ladder.exe。",
        "The last application update did not finish. Your configuration was preserved. Retry the update, or exit and replace ladder.exe manually.",
        "Последнее обновление приложения не завершено. Конфигурация сохранена. Повторите обновление или закройте приложение и замените ladder.exe вручную."
    );
    app_update_busy => (
        "正在处理软件更新",
        "An application update is in progress",
        "Выполняется обновление приложения"
    );
    check_app_updates_first => (
        "请先检查软件更新",
        "Check for application updates first",
        "Сначала проверьте обновления приложения"
    );
    app_update_not_ready => (
        "软件更新尚未准备好",
        "The application update is not ready yet",
        "Обновление приложения ещё не готово"
    );
    app_update_unavailable => (
        "软件更新文件不可用, 请重新下载",
        "The application update file is unavailable. Download it again",
        "Файл обновления приложения недоступен. Загрузите его ещё раз"
    );
    invalid_core_release => (
        "官方发布信息格式无效, 请稍后重试",
        "The official release information is invalid. Try again later",
        "Некорректный формат сведений об официальном выпуске. Повторите позже"
    );
    core_release_not_installable => (
        "官方响应不是可安装的版本",
        "The official response does not describe an installable version",
        "Официальный ответ не содержит версию, доступную для установки"
    );
    core_release_not_published => (
        "官方响应不是已发布的可安装版本",
        "The official response does not describe a published, installable version",
        "Официальный ответ не содержит опубликованную версию, доступную для установки"
    );
    missing_core_architecture => (
        "官方发布版本缺少当前架构的 Windows 安装包",
        "The official release has no Windows package for the current architecture",
        "В официальном выпуске нет пакета Windows для текущей архитектуры"
    );
    duplicate_core_package => (
        "官方发布信息包含重复的安装包",
        "The official release information contains duplicate packages",
        "Сведения об официальном выпуске содержат дублирующиеся пакеты"
    );
    unofficial_core_package_url => (
        "安装包下载地址不是对应的官方发布地址",
        "The package download URL does not match the official release URL",
        "Адрес загрузки пакета не соответствует адресу официального выпуска"
    );
    invalid_core_package_size => (
        "官方安装包大小无效或超出限制",
        "The official package size is invalid or exceeds the limit",
        "Размер официального пакета недопустим или превышает ограничение"
    );
    missing_core_digest => (
        "官方安装包缺少有效的 SHA-256 校验值",
        "The official package has no valid SHA-256 checksum",
        "Для официального пакета отсутствует корректная контрольная сумма SHA-256"
    );
    invalid_core_release_list => (
        "官方发布列表格式无效, 请稍后重试",
        "The official release list is invalid. Try again later",
        "Некорректный формат списка официальных выпусков. Повторите позже"
    );
    invalid_versions_page => (
        "版本列表页码无效",
        "Invalid version list page number",
        "Недопустимый номер страницы списка версий"
    );
    invalid_core_version => (
        "核心版本号无效",
        "Invalid core version",
        "Недопустимый номер версии ядра"
    );
    invalid_core_directory => (
        "核心版本路径不是普通目录, 无法执行操作",
        "The core version path is not a regular directory. The operation cannot continue",
        "Путь версии ядра не является обычным каталогом. Операция невозможна"
    );
    delete_outside_core_directory => (
        "待删除版本不在核心版本目录内",
        "The version to delete is outside the core versions directory",
        "Удаляемая версия находится вне каталога версий ядра"
    );
    core_request_limited => (
        "GitHub 暂时限制请求, 请稍后重试",
        "GitHub is temporarily rate-limiting requests. Try again later",
        "GitHub временно ограничивает частоту запросов. Повторите позже"
    );
    core_release_unavailable => (
        "官方发布文件暂不可用, 请稍后重试",
        "The official release file is temporarily unavailable. Try again later",
        "Файл официального выпуска временно недоступен. Повторите позже"
    );
    core_download_timeout => (
        "连接或下载超时, 请检查网络后重试",
        "The connection or download timed out. Check your connection and try again",
        "Превышено время ожидания соединения или загрузки. Проверьте соединение и повторите попытку"
    );
    core_request_failed => (
        "无法连接官方发布服务, 请检查网络、代理和证书后重试",
        "Cannot connect to the official release service. Check your network, proxy and certificates, then try again",
        "Не удалось подключиться к сервису официальных выпусков. Проверьте сеть, прокси и сертификаты, затем повторите попытку"
    );
    update_cancelled => (
        "已取消更新",
        "Update cancelled",
        "Обновление отменено"
    );
    core_version_exists => (
        "该版本已存在于本地, 请先删除后再下载",
        "This version already exists locally. Delete it before downloading again",
        "Эта версия уже есть на компьютере. Удалите её перед повторной загрузкой"
    );
    refresh_versions_first => (
        "请先刷新官方版本列表",
        "Refresh the official version list first",
        "Сначала обновите список официальных версий"
    );
    core_tag_mismatch => (
        "官方版本标签与版本号不符, 请刷新列表",
        "The official release tag does not match the version. Refresh the list",
        "Тег официального выпуска не соответствует версии. Обновите список"
    );
    core_release_mismatch => (
        "官方发布响应与所选版本不符, 请刷新列表",
        "The official release response does not match the selected version. Refresh the list",
        "Ответ об официальном выпуске не соответствует выбранной версии. Обновите список"
    );
    core_package_version_mismatch => (
        "安装包中的核心版本与官方发布信息不符, 现有核心未修改",
        "The core version in the package does not match the official release information. Existing cores are unchanged",
        "Версия ядра в пакете не соответствует сведениям об официальном выпуске. Установленные ядра не изменены"
    );
    core_download_interrupted => (
        "下载中断或超时, 请检查网络后重试",
        "The download was interrupted or timed out. Check your connection and try again",
        "Загрузка прервана или превышено время ожидания. Проверьте соединение и повторите попытку"
    );
    core_download_size_mismatch => (
        "下载大小与官方发布信息不符, 请重试",
        "The download size does not match the official release information. Try again",
        "Размер загрузки не соответствует сведениям об официальном выпуске. Повторите попытку"
    );
    core_download_incomplete => (
        "安装包下载不完整, 请重试",
        "The package download is incomplete. Try again",
        "Пакет загружен не полностью. Повторите попытку"
    );
    core_digest_mismatch => (
        "安装包 SHA-256 校验失败, 现有核心未修改, 请重试",
        "The package SHA-256 checksum is incorrect. Existing cores are unchanged. Try again",
        "Контрольная сумма SHA-256 пакета неверна. Установленные ядра не изменены. Повторите попытку"
    );
    invalid_core_zip => (
        "安装包不是有效的 ZIP 文件",
        "The package is not a valid ZIP archive",
        "Пакет не является корректным ZIP-архивом"
    );
    cannot_read_archive_entry => (
        "无法读取安装包条目",
        "Cannot read a package entry",
        "Не удалось прочитать элемент пакета"
    );
    invalid_archive_filename => (
        "安装包包含无效文件名",
        "The package contains an invalid file name",
        "Пакет содержит недопустимое имя файла"
    );
    unsafe_archive_path => (
        "安装包包含不安全路径或符号链接",
        "The package contains an unsafe path or a symbolic link",
        "Пакет содержит небезопасный путь или символическую ссылку"
    );
    archive_extraction_limit => (
        "安装包解压大小超出限制",
        "The extracted package size exceeds the limit",
        "Размер распакованного пакета превышает ограничение"
    );
    archive_extraction_failed => (
        "安装包解压失败或内容损坏",
        "The package could not be extracted or is corrupted",
        "Не удалось распаковать пакет или его содержимое повреждено"
    );
    archive_entry_size_mismatch => (
        "安装包解压大小与条目信息不符",
        "The extracted size does not match the package entry information",
        "Размер распакованных данных не соответствует сведениям об элементе пакета"
    );
    archive_entry_incomplete => (
        "安装包条目不完整",
        "The package entry is incomplete",
        "Элемент пакета неполон"
    );
    core_download_complete => (
        "该版本已完成下载",
        "This version has already been downloaded",
        "Эта версия уже загружена"
    );
    version_record_conflict => (
        "安装包包含与版本记录冲突的 .ladder-version.json",
        "The package contains a conflicting .ladder-version.json file",
        "Пакет содержит конфликтующий файл .ladder-version.json"
    );
    cannot_run_version_command => (
        "无法运行核心版本命令, 可重新安装官方核心",
        "Cannot run the core version command. Try reinstalling the official core",
        "Не удалось выполнить команду проверки версии ядра. Попробуйте переустановить официальное ядро"
    );
    version_command_failed => (
        "核心版本命令失败, 可重新安装官方核心",
        "The core version command failed. Try reinstalling the official core",
        "Ошибка команды проверки версии ядра. Попробуйте переустановить официальное ядро"
    );
    cannot_read_version_status => (
        "无法读取核心版本命令状态",
        "Cannot read the core version command status",
        "Не удалось прочитать состояние команды проверки версии ядра"
    );
    version_command_timeout => (
        "读取核心版本超时, 可重新安装官方核心",
        "Reading the core version timed out. Try reinstalling the official core",
        "Превышено время ожидания версии ядра. Попробуйте переустановить официальное ядро"
    );
    invalid_version_output => (
        "核心版本命令输出异常, 可重新安装官方核心",
        "The core version command returned invalid output. Try reinstalling the official core",
        "Команда проверки версии ядра вернула некорректные данные. Попробуйте переустановить официальное ядро"
    );
    version_process_still_running => (
        "版本检测进程尚未完全退出, 请重试",
        "The version check process has not fully exited. Try again",
        "Процесс проверки версии ещё не завершён. Повторите попытку"
    );
    cannot_read_core_version => (
        "无法读取核心版本",
        "Cannot read the core version",
        "Не удалось прочитать версию ядра"
    );
    unrecognized_core_version => (
        "无法识别核心版本, 可重新安装官方核心",
        "Cannot identify the core version. Try reinstalling the official core",
        "Не удалось определить версию ядра. Попробуйте переустановить официальное ядро"
    );
    invalid_core_executable => (
        "核心不是有效的 Windows 程序, 可重新安装官方核心",
        "The core is not a valid Windows executable. Try reinstalling the official core",
        "Ядро не является корректным исполняемым файлом Windows. Попробуйте переустановить официальное ядро"
    );
    cannot_access_directory(error: impl std::fmt::Display) => (
        "无法访问目录: {error}",
        "Cannot access the directory: {error}",
        "Не удалось получить доступ к каталогу: {error}"
    );
    cannot_open_directory(result: impl std::fmt::Display) => (
        "无法打开目录 (Windows 错误码 {result})",
        "Cannot open the directory (Windows error {result})",
        "Не удалось открыть каталог (код ошибки Windows: {result})"
    );
    cannot_access_config(error: impl std::fmt::Display) => (
        "无法访问配置文件: {error}",
        "Cannot access the configuration file: {error}",
        "Не удалось получить доступ к файлу конфигурации: {error}"
    );
    cannot_open_editor(error: impl std::fmt::Display) => (
        "无法打开配置编辑器 (Windows 错误码 {error})",
        "Cannot open the configuration editor (Windows error {error})",
        "Не удалось открыть редактор конфигурации (код ошибки Windows: {error})"
    );
    cannot_locate_notepad(error: impl std::fmt::Display) => (
        "无法定位系统记事本: {error}",
        "Cannot locate Notepad: {error}",
        "Не удалось найти Блокнот: {error}"
    );
    cannot_start_notepad(error: impl std::fmt::Display) => (
        "无法启动系统记事本 (Windows 错误码 {error})",
        "Cannot start Notepad (Windows error {error})",
        "Не удалось запустить Блокнот (код ошибки Windows: {error})"
    );
    cannot_start_app_check(error: impl std::fmt::Display) => (
        "无法创建软件更新检查线程: {error}",
        "Cannot start the application update check thread: {error}",
        "Не удалось запустить поток проверки обновлений приложения: {error}"
    );
    cannot_start_app_download(error: impl std::fmt::Display) => (
        "无法创建软件下载线程: {error}",
        "Cannot start the application download thread: {error}",
        "Не удалось запустить поток загрузки приложения: {error}"
    );
    cannot_check_core_directory(error: impl std::fmt::Display) => (
        "无法检查核心版本目录: {error}",
        "Cannot inspect the core version directory: {error}",
        "Не удалось проверить каталог версии ядра: {error}"
    );
    cannot_read_local_versions(error: impl std::fmt::Display) => (
        "无法读取本地核心版本: {error}",
        "Cannot read local core versions: {error}",
        "Не удалось прочитать локальные версии ядра: {error}"
    );
    cannot_check_local_versions(error: impl std::fmt::Display) => (
        "无法检查本地核心版本: {error}",
        "Cannot inspect local core versions: {error}",
        "Не удалось проверить локальные версии ядра: {error}"
    );
    cannot_read_version_record(error: impl std::fmt::Display) => (
        "无法读取核心版本记录: {error}",
        "Cannot read the core version record: {error}",
        "Не удалось прочитать запись о версии ядра: {error}"
    );
    cannot_locate_core_directory(error: impl std::fmt::Display) => (
        "无法定位核心版本目录: {error}",
        "Cannot locate the core versions directory: {error}",
        "Не удалось найти каталог версий ядра: {error}"
    );
    cannot_locate_deleted_version(error: impl std::fmt::Display) => (
        "无法定位待删除版本: {error}",
        "Cannot locate the version to delete: {error}",
        "Не удалось найти удаляемую версию: {error}"
    );
    cannot_delete_core_version(version: impl std::fmt::Display, error: impl std::fmt::Display) => (
        "删除核心 {version} 失败, 请关闭占用该版本的程序后重试: {error}",
        "Failed to delete core {version}. Close programs using this version and try again: {error}",
        "Не удалось удалить ядро {version}. Закройте программы, использующие эту версию, и повторите попытку: {error}"
    );
    download_http_error(code: impl std::fmt::Display) => (
        "下载服务返回 HTTP {code}, 请稍后重试",
        "The download service returned HTTP {code}. Try again later",
        "Сервис загрузки вернул HTTP {code}. Повторите позже"
    );
    package_validation_failed(message: impl std::fmt::Display) => (
        "安装包验证失败: {message}；现有核心未修改",
        "Package validation failed: {message}; existing cores are unchanged",
        "Ошибка проверки пакета: {message}; установленные ядра не изменены"
    );
    cannot_create_download_file(e: impl std::fmt::Display) => (
        "创建下载临时文件失败: {e}",
        "Failed to create a temporary download file: {e}",
        "Не удалось создать временный файл загрузки: {e}"
    );
    cannot_save_download(e: impl std::fmt::Display) => (
        "保存下载文件失败: {e}",
        "Failed to save the downloaded file: {e}",
        "Не удалось сохранить загруженный файл: {e}"
    );
    cannot_create_download_stage(e: impl std::fmt::Display) => (
        "创建下载暂存目录失败: {e}",
        "Failed to create the download staging directory: {e}",
        "Не удалось создать временный каталог загрузки: {e}"
    );
    cannot_open_package(e: impl std::fmt::Display) => (
        "打开安装包失败: {e}",
        "Failed to open the package: {e}",
        "Не удалось открыть пакет: {e}"
    );
    cannot_create_stage_directory(e: impl std::fmt::Display) => (
        "创建暂存目录失败: {e}",
        "Failed to create the staging directory: {e}",
        "Не удалось создать временный каталог: {e}"
    );
    cannot_create_stage_file(e: impl std::fmt::Display) => (
        "创建暂存文件失败: {e}",
        "Failed to create a staging file: {e}",
        "Не удалось создать временный файл: {e}"
    );
    cannot_write_stage_file(e: impl std::fmt::Display) => (
        "写入暂存文件失败: {e}",
        "Failed to write to a staging file: {e}",
        "Не удалось записать временный файл: {e}"
    );
    cannot_save_stage_file(e: impl std::fmt::Display) => (
        "保存暂存文件失败: {e}",
        "Failed to save a staging file: {e}",
        "Не удалось сохранить временный файл: {e}"
    );
    cannot_move_mihomo(error: impl std::fmt::Display) => (
        "整理 mihomo 程序文件失败: {error}",
        "Failed to arrange the mihomo executable: {error}",
        "Не удалось разместить исполняемый файл mihomo: {error}"
    );
    cannot_read_stage_directory(e: impl std::fmt::Display) => (
        "读取暂存目录失败: {e}",
        "Failed to read the staging directory: {e}",
        "Не удалось прочитать временный каталог: {e}"
    );
    missing_core_executable(executable: impl std::fmt::Display) => (
        "安装包缺少 {executable}",
        "The package does not contain {executable}",
        "В пакете отсутствует {executable}"
    );
    cannot_read_staged_core(e: impl std::fmt::Display) => (
        "读取暂存核心信息失败: {e}",
        "Failed to read the staged core information: {e}",
        "Не удалось прочитать сведения о подготовленном ядре: {e}"
    );
    empty_core_executable(executable: impl std::fmt::Display) => (
        "安装包中的 {executable} 为空",
        "The {executable} file in the package is empty",
        "Файл {executable} в пакете пуст"
    );
    cannot_create_version_record(error: impl std::fmt::Display) => (
        "创建版本记录失败: {error}",
        "Failed to create the version record: {error}",
        "Не удалось создать запись о версии: {error}"
    );
    cannot_save_version_record(error: impl std::fmt::Display) => (
        "保存版本记录失败: {error}",
        "Failed to save the version record: {error}",
        "Не удалось сохранить запись о версии: {error}"
    );
    cannot_create_core_directory(error: impl std::fmt::Display) => (
        "创建核心版本目录失败: {error}",
        "Failed to create the core version directory: {error}",
        "Не удалось создать каталог версии ядра: {error}"
    );
    cannot_save_core_version(error: impl std::fmt::Display) => (
        "保存核心版本失败, 现有版本未修改: {error}",
        "Failed to save the core version. Existing versions are unchanged: {error}",
        "Не удалось сохранить версию ядра. Установленные версии не изменены: {error}"
    );
    cannot_create_version_tempfile(e: impl std::fmt::Display) => (
        "创建版本检测临时文件失败: {e}",
        "Failed to create a temporary file for version detection: {e}",
        "Не удалось создать временный файл проверки версии: {e}"
    );
}
