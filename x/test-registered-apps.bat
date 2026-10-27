@echo off
REM Test every install method for every app in config\registered-apps.json.
REM
REM For each app the script tries, in order:
REM   global   cargo install --force <crate_name>
REM   git      cargo install --force --git <repository>
REM   local    git clone <repository> into downloads\ + cargo build --release
REM
REM Methods that succeed are written back to "install_methods" in
REM config\registered-apps.json, methods that fail to "failed_install_methods".
REM An app where no method works is flagged in logs\test-registered-apps.log.
REM
REM Usage: x\test-registered-apps.bat [--app <key>] [--keep]

setlocal enabledelayedexpansion

set "SCRIPT_DIR=%~dp0"
for %%I in ("%SCRIPT_DIR%..") do set "PROJECT_ROOT=%%~fI"
set "REGISTERED_JSON=%PROJECT_ROOT%\config\registered-apps.json"
set "LOG_DIR=%PROJECT_ROOT%\logs"
set "LOG_FILE=%LOG_DIR%\test-registered-apps.log"
set "DOWNLOADS_DIR=%PROJECT_ROOT%\downloads"
set "RESULTS=%TEMP%\tuios-install-test-%RANDOM%.txt"
set "APP_LIST=%TEMP%\tuios-install-apps-%RANDOM%.txt"

set "ONLY_APP="
set "KEEP_CLONES=0"

:parse_args
if "%~1"=="" goto args_done
if /i "%~1"=="--app" (
    if "%~2"=="" (
        echo --app needs an app key
        exit /b 1
    )
    set "ONLY_APP=%~2"
    shift
    shift
    goto parse_args
)
if /i "%~1"=="--keep" (
    set "KEEP_CLONES=1"
    shift
    goto parse_args
)
if /i "%~1"=="-h" goto usage
if /i "%~1"=="--help" goto usage
echo Unknown option: %~1
goto usage
:args_done

set "PY="
where python >nul 2>&1 && set "PY=python"
if not defined PY (
    where py >nul 2>&1 && set "PY=py -3"
)
if not defined PY (
    echo Python 3 is required.
    exit /b 1
)
where cargo >nul 2>&1
if errorlevel 1 (
    echo cargo is required.
    exit /b 1
)
where git >nul 2>&1
if errorlevel 1 (
    echo git is required.
    exit /b 1
)
if not exist "%REGISTERED_JSON%" (
    echo Not found: %REGISTERED_JSON%
    exit /b 1
)

if not exist "%LOG_DIR%" mkdir "%LOG_DIR%"
if exist "%RESULTS%" del /q "%RESULTS%"
if exist "%APP_LIST%" del /q "%APP_LIST%"

echo === tuiOS Registered Apps Install Test ===
echo Log: %LOG_FILE%
echo.
call :log SCRIPT "test-registered-apps.bat started"

set "TOTAL=0"
set "FLAGGED=0"

%PY% -c "import json,sys; d=json.load(open(sys.argv[1],encoding='utf-8')); only=sys.argv[2]; print('\n'.join('#'.join([k,(m.get('crate_name') or k),(m.get('repository') or ''),(m.get('category') or 'Other')]) for k,m in d.items() if not only or k==only))" "%REGISTERED_JSON%" "%ONLY_APP%" > "%APP_LIST%"
if errorlevel 1 (
    echo Failed to read %REGISTERED_JSON%
    exit /b 1
)

for /f "usebackq tokens=1-4 delims=#" %%A in ("%APP_LIST%") do (
    call :test_app "%%A" "%%B" "%%C" "%%D"
)
del /q "%APP_LIST%" >nul 2>&1

if "%TOTAL%"=="0" (
    echo No apps to test in %REGISTERED_JSON%
    if exist "%RESULTS%" del /q "%RESULTS%"
    exit /b 1
)

%PY% -c "import json,sys; p=sys.argv[1]; d=json.load(open(p,encoding='utf-8')); f=lambda s: [] if s=='-' else [x for x in s.split(',') if x]; rows=[l.strip().split('#') for l in open(sys.argv[2],encoding='utf-8') if l.strip()]; [d[r[0]].update({'install_methods':f(r[1]),'failed_install_methods':f(r[2])}) for r in rows if r[0] in d]; open(p,'w',encoding='utf-8').write(json.dumps(d,indent=2,ensure_ascii=False)+'\n')" "%REGISTERED_JSON%" "%RESULTS%"

echo === Summary ===
for /f "usebackq tokens=1-3 delims=#" %%A in ("%RESULTS%") do echo   %%A: works=%%B failed=%%C
echo.
echo Apps tested: %TOTAL%, flagged ^(no working method^): %FLAGGED%
call :log SCRIPT "test-registered-apps.bat completed - %TOTAL% tested, %FLAGGED% flagged"
echo Updated: %REGISTERED_JSON%

del /q "%RESULTS%" >nul 2>&1
if not "%FLAGGED%"=="0" exit /b 1
exit /b 0

REM --------------------------------------------------------------------------

:usage
echo Usage: test-registered-apps.bat [--app ^<key^>] [--keep]
echo   --app ^<key^>  Only test the given registered-apps.json key
echo   --keep       Keep directories the local method cloned into downloads\
exit /b 1

:test_app
set "KEY=%~1"
set "CRATE=%~2"
set "REPO=%~3"
set "CATEGORY=%~4"
set "WORKING="
set "FAILED="
set /a TOTAL+=1

echo --- !KEY! ^(!CATEGORY!^) ---
if defined REPO (
    call :log SCRIPT "Testing !KEY! crate=!CRATE! repo=!REPO!"
) else (
    call :log SCRIPT "Testing !KEY! crate=!CRATE! repo=none"
)

call :method_global
call :method_git
call :method_local

if not defined WORKING set "WORKING=-"
if not defined FAILED set "FAILED=-"
(echo !KEY!#!WORKING!#!FAILED!)>>"%RESULTS%"

if "!WORKING!"=="-" (
    set /a FLAGGED+=1
    echo   FLAGGED: no install method works for !KEY!
    call :log FLAG "No install method works for '!KEY!' - failed: !FAILED!"
)
echo.
goto :eof

:method_global
echo   - global: running ...
call :log CMD "cargo install --force !CRATE!"
cargo install --force "!CRATE!" >>"%LOG_FILE%" 2>&1
if errorlevel 1 (call :mark_failed global) else (call :mark_working global)
goto :eof

:method_git
if not defined REPO (
    echo   - git: skipped, no repository
    call :log SCRIPT "!KEY!: git skipped, no repository"
    goto :eof
)
echo   - git: running ...
call :log CMD "cargo install --force --git !REPO!"
cargo install --force --git "!REPO!" >>"%LOG_FILE%" 2>&1
if errorlevel 1 (call :mark_failed git) else (call :mark_working git)
goto :eof

:method_local
if not defined REPO (
    echo   - local: skipped, no repository
    call :log SCRIPT "!KEY!: local skipped, no repository"
    goto :eof
)
echo   - local: running ...
set "SUBDIR=applications"
if /i "!CATEGORY!"=="Dashboard" set "SUBDIR=dashboards"
set "TARGET=%DOWNLOADS_DIR%\!SUBDIR!\!KEY!"
set "FRESH=0"
if not exist "%DOWNLOADS_DIR%\!SUBDIR!" mkdir "%DOWNLOADS_DIR%\!SUBDIR!"

if exist "!TARGET!" (
    if exist "!TARGET!\.git" (
        call :log CMD "git -C !TARGET! pull"
        git -C "!TARGET!" pull >>"%LOG_FILE%" 2>&1
        if errorlevel 1 (
            call :mark_failed local
            goto :eof
        )
    )
) else (
    set "FRESH=1"
    call :log CMD "git clone !REPO! !TARGET!"
    git clone "!REPO!" "!TARGET!" >>"%LOG_FILE%" 2>&1
    if errorlevel 1 (
        rmdir /s /q "!TARGET!" >nul 2>&1
        call :mark_failed local
        goto :eof
    )
)

REM The app store strips VCS metadata from downloads\ copies.
if exist "!TARGET!\.git" rmdir /s /q "!TARGET!\.git" >nul 2>&1
if exist "!TARGET!\.github" rmdir /s /q "!TARGET!\.github" >nul 2>&1

call :log CMD "cargo build --release --manifest-path !TARGET!\Cargo.toml"
cargo build --release --manifest-path "!TARGET!\Cargo.toml" >>"%LOG_FILE%" 2>&1
if errorlevel 1 (set "LOCAL_OK=0") else (set "LOCAL_OK=1")

if "!FRESH!"=="1" if "%KEEP_CLONES%"=="0" rmdir /s /q "!TARGET!" >nul 2>&1

if "!LOCAL_OK!"=="1" (call :mark_working local) else (call :mark_failed local)
goto :eof

:mark_working
if defined WORKING (set "WORKING=!WORKING!,%~1") else (set "WORKING=%~1")
echo     OK
call :log RESULT "!KEY!: %~1 OK"
goto :eof

:mark_failed
if defined FAILED (set "FAILED=!FAILED!,%~1") else (set "FAILED=%~1")
echo     FAILED
call :log RESULT "!KEY!: %~1 FAILED"
goto :eof

:log
for /f "usebackq tokens=1-2" %%a in (`powershell -NoProfile -Command "Get-Date -Format 'yyyy-MM-dd HH:mm:ss'"`) do set "TS=%%a %%b"
(echo [!TS!] [%~1] %~2)>>"%LOG_FILE%"
goto :eof
