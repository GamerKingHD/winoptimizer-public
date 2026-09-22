@echo off
setlocal enabledelayedexpansion

set "PROJECT_ROOT=%~dp0"
cd /d "%PROJECT_ROOT%"
set "BUILD_DIR=%PROJECT_ROOT%.build"
set "CACHE_ROOT=%WINOPTIMIZER_CACHE_ROOT%"
if not defined CACHE_ROOT if defined RUNNER_TEMP set "CACHE_ROOT=%RUNNER_TEMP%\WinOptimizer"
if not defined CACHE_ROOT if defined LOCALAPPDATA set "CACHE_ROOT=%LOCALAPPDATA%\WinOptimizer\build-cache"
if not defined CACHE_ROOT set "CACHE_ROOT=%TEMP%\WinOptimizer"
if defined WINOPTIMIZER_TARGET_DIR (
    set "CARGO_TARGET_DIR=%WINOPTIMIZER_TARGET_DIR%"
) else (
    set "CARGO_TARGET_DIR=%CACHE_ROOT%\cargo-target"
)

:: Defaults
set "PROFILE=release-fast"
set "CLEAN_FIRST="
set "CHECK_ONLY="
set "NIGHTLY="
set "FRONTEND_ONLY="
set "MSI_ONLY="
set "SHOW_STATS="

:: Parse flags
:parse
if "%~1"=="" goto :dispatch
if /i "%~1"=="--clean"       set "CLEAN_FIRST=1"   & shift & goto :parse
if /i "%~1"=="--release"     set "PROFILE=release" & shift & goto :parse
if /i "%~1"=="--dev"         set "PROFILE=dev"     & shift & goto :parse
if /i "%~1"=="--check"       set "CHECK_ONLY=1"    & shift & goto :parse
if /i "%~1"=="--nightly"     set "NIGHTLY=1"       & shift & goto :parse
if /i "%~1"=="--frontend"    set "FRONTEND_ONLY=1"  & shift & goto :parse
if /i "%~1"=="--msi"        set "MSI_ONLY=1"      & shift & goto :parse
if /i "%~1"=="--stats"       set "SHOW_STATS=1"    & shift & goto :parse
if /i "%~1"=="-h"            goto :help
if /i "%~1"=="--help"        goto :help
echo Unknown flag: %~1 && exit /b 1

:: Help
:help
echo.
echo   build.bat [flags]
echo.
echo   Inner loop
echo   npm run tauri dev     Vite HMR + cargo --profile dev. Use this for 1-2 line edits.
echo   cargo run --manifest-path src-tauri\Cargo.toml
echo   Do not use npm run build / release-fast for inner edits. Those are packaged smoke only.
echo.
echo   Modes
echo   (default)      Incremental release-fast build for packaged smoke only
echo   --release      Full optimized build (lto + single codegen unit)
echo   --dev          Debug build (fastest compile, no optimizations)
echo   --check        cargo check only - verify code compiles (fastest)
echo   --frontend     Build frontend only (.build\frontend)
echo   --msi          Build the MSI installer only (requires WiX; no acceptance claim)
echo.
echo   Options
echo   --clean        Remove target dir before building (fresh start)
echo   --nightly      Use nightly toolchain with -Z threads=0
echo   --stats        Show sccache hit/miss stats after build
echo.
echo   Cache root: %CACHE_ROOT%
echo   Target root: %CARGO_TARGET_DIR%
echo   Override the cache root with WINOPTIMIZER_CACHE_ROOT when needed.
echo   Override target root with WINOPTIMIZER_TARGET_DIR for reproducible benchmarks.
echo.
echo   Profiles
echo   dev           opt=0  debug=full   codegen=256   ThinLTO   ~3s compile
echo   release-fast  opt=2  debug=none   codegen=16    no LTO    ~15s compile
echo   release       opt=3  debug=none   codegen=1     full LTO  ~60s compile
echo.
exit /b 0

:: Frontend only
:frontend
echo [1/1] Building frontend...
call npm run build:frontend
if errorlevel 1 goto :fail
echo Frontend: .build\frontend\
exit /b 0

:: Dispatch
:dispatch
if "%FRONTEND_ONLY%"=="1" goto :frontend
if "%MSI_ONLY%"=="1" goto :msi

:: MSI packaging is explicit and separate from the default portable path.
:msi
if "%MSI_ONLY%"=="1" (
    echo MSI packaging requires the WiX toolchain and a separate Phase 39 acceptance run.
    call node tools\release\installer\build-msi.mjs
    if errorlevel 1 goto :fail
    exit /b 0
)

:: Toolchain
set "CARGO_CMD=cargo"
set "RUSTFLAGS_PRE="
if "%NIGHTLY%"=="1" (
    set "CARGO_CMD=cargo +nightly"
    set "RUSTFLAGS_PRE=-Z threads=0"
    echo Using nightly toolchain with parallel frontend
)

:: sccache
:: Keep compiler cache outside the checkout and bounded by sccache.
set "SCCACHE_DIR=%CACHE_ROOT%\sccache"
set "SCCACHE_CACHE_SIZE=10G"
if not exist "%CACHE_ROOT%" mkdir "%CACHE_ROOT%"
if errorlevel 1 goto :fail
if not exist "%CARGO_TARGET_DIR%" mkdir "%CARGO_TARGET_DIR%"
if errorlevel 1 goto :fail
if not exist "%SCCACHE_DIR%" mkdir "%SCCACHE_DIR%"
if errorlevel 1 goto :fail

echo.
echo === WinOptimizer build ===
echo Profile: %PROFILE%
if "%NIGHTLY%"=="1"    echo Frontend: nightly parallel
if "%CLEAN_FIRST%"=="1" echo Mode:    clean build

:: Clean (opt-in)
if "%CLEAN_FIRST%"=="1" (
    echo.
    echo [clean] Removing target dir for fresh build...
    if exist "%CARGO_TARGET_DIR%" rmdir /S /Q "%CARGO_TARGET_DIR%"
    if exist dist rmdir /S /Q dist
)

:: Frontend
echo.
echo [1/2] Building frontend...
call npm run build:frontend
if errorlevel 1 goto :fail

:: Ensure clean dist dir
if not "%CHECK_ONLY%"=="1" (
    if exist dist rmdir /S /Q dist
    mkdir dist
)

:: Cargo build
echo.
if "%CHECK_ONLY%"=="1" (
    echo [2/2] cargo check --profile %PROFILE%
    %CARGO_CMD% check --locked --manifest-path src-tauri\Cargo.toml --profile %PROFILE% --target-dir "%CARGO_TARGET_DIR%"
) else (
    echo [2/2] cargo build --profile %PROFILE%
    if "%NIGHTLY%"=="1" (
        set "RUSTFLAGS=%RUSTFLAGS_PRE%"
        %CARGO_CMD% build --locked --manifest-path src-tauri\Cargo.toml --profile %PROFILE% --target-dir "%CARGO_TARGET_DIR%"
    ) else (
        %CARGO_CMD% build --locked --manifest-path src-tauri\Cargo.toml --profile %PROFILE% --target-dir "%CARGO_TARGET_DIR%"
    )
)
if errorlevel 1 goto :fail

:: Stats
if "%SHOW_STATS%"=="1" (
    echo.
    echo --- sccache stats ---
    sccache --show-stats 2>nul || echo sccache not found
)

:: Skip copy for check-only
if "%CHECK_ONLY%"=="1" (
    echo.
    echo Check passed. No binary produced.
    exit /b 0
)

:: Copy output
echo.
if "%PROFILE%"=="dev" (
    copy /Y "%CARGO_TARGET_DIR%\debug\winoptimizer.exe" dist\winoptimizer.exe > nul
) else if "%PROFILE%"=="release-fast" (
    copy /Y "%CARGO_TARGET_DIR%\release-fast\winoptimizer.exe" dist\winoptimizer.exe > nul
) else (
    copy /Y "%CARGO_TARGET_DIR%\release\winoptimizer.exe" dist\winoptimizer.exe > nul
)
if errorlevel 1 goto :fail

echo Portable executable: dist\winoptimizer.exe
echo Build complete.
exit /b 0

:: Failure
:fail
set "EXIT_CODE=%ERRORLEVEL%"
echo Build FAILED (exit code %EXIT_CODE%)
exit /b %EXIT_CODE%
