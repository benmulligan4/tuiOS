@echo off
REM Test every install method for every app in config\registered-apps.json.
REM
REM Nothing is installed for real: every method runs against a throwaway sandbox
REM directory that is deleted afterwards, so %%USERPROFILE%%\.cargo\bin and
REM downloads\ are left exactly as they were.
REM
REM For each app the script tries, in order:
REM   global   cargo install --root <sandbox> <crate_name>
REM   git      cargo install --root <sandbox> --git <repository>
REM   local    git clone <repository> into <sandbox> + cargo build --release
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
set "RESULTS=%TEMP%\tuios-install-test-%RANDOM%.txt"
set "APP_LIST=%TEMP%\tuios-install-apps-%RANDOM%.txt"
set "SANDBOX=%TEMP%\tuios-install-sandbox-%RANDOM%"

set "ONLY_APP="
set "KEEP_SANDBOX=0"

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
    set "KEEP_SANDBOX=1"
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
if not exist "%SANDBOX%" mkdir "%SANDBOX%"

echo === tuiOS Registered Apps Install Test ===
echo Log:     %LOG_FILE%
echo Sandbox: %SANDBOX% - nothing is installed outside it
echo.
call :log SCRIPT "test-registered-apps.bat started, sandbox %SANDBOX%"

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
    rmdir /s /q "%SANDBOX%" >nul 2>&1
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
if "%KEEP_SANDBOX%"=="1" (
    echo Sandbox kept at: %SANDBOX%
) else (
    rmdir /s /q "%SANDBOX%" >nul 2>&1
)
if not "%FLAGGED%"=="0" exit /b 1
exit /b 0

REM --------------------------------------------------------------------------

:usage
echo Usage: test-registered-apps.bat [--app ^<key^>] [--keep]
echo   --app ^<key^>  Only test the given registered-apps.json key
echo   --keep       Keep the sandbox directory instead of deleting it
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
set "ROOT=%SANDBOX%\root-!KEY!-global"
call :log CMD "cargo install --root !ROOT! !CRATE!"
cargo install --root "!ROOT!" "!CRATE!" >>"%LOG_FILE%" 2>&1
if errorlevel 1 (call :mark_failed global) else (call :mark_working global)
if "%KEEP_SANDBOX%"=="0" rmdir /s /q "!ROOT!" >nul 2>&1
goto :eof

:method_git
if not defined REPO (
    echo   - git: skipped, no repository
    call :log SCRIPT "!KEY!: git skipped, no repository"
    goto :eof
)
echo   - git: running ...
set "ROOT=%SANDBOX%\root-!KEY!-git"
call :log CMD "cargo install --root !ROOT! --git !REPO!"
cargo install --root "!ROOT!" --git "!REPO!" >>"%LOG_FILE%" 2>&1
if errorlevel 1 (call :mark_failed git) else (call :mark_working git)
if "%KEEP_SANDBOX%"=="0" rmdir /s /q "!ROOT!" >nul 2>&1
goto :eof

:method_local
if not defined REPO (
    echo   - local: skipped, no repository
    call :log SCRIPT "!KEY!: local skipped, no repository"
    goto :eof
)
echo   - local: running ...
set "TARGET=%SANDBOX%\clone-!KEY!"
call :log CMD "git clone !REPO! !TARGET!"
git clone "!REPO!" "!TARGET!" >>"%LOG_FILE%" 2>&1
if errorlevel 1 (
    rmdir /s /q "!TARGET!" >nul 2>&1
    call :mark_failed local
    goto :eof
)

REM The app store strips VCS metadata from downloads\ copies.
if exist "!TARGET!\.git" rmdir /s /q "!TARGET!\.git" >nul 2>&1
if exist "!TARGET!\.github" rmdir /s /q "!TARGET!\.github" >nul 2>&1

call :log CMD "cargo build --release --manifest-path !TARGET!\Cargo.toml"
cargo build --release --manifest-path "!TARGET!\Cargo.toml" >>"%LOG_FILE%" 2>&1
if errorlevel 1 (call :mark_failed local) else (call :mark_working local)
if "%KEEP_SANDBOX%"=="0" rmdir /s /q "!TARGET!" >nul 2>&1
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
