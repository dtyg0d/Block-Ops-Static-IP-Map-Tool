@echo off
title Build BlockOps Rust EXE
echo.
echo Building BlockOps Static IP Tool in Rust...
echo.

where cargo >nul 2>nul
if %errorlevel% neq 0 (
    echo Rust/Cargo was not found.
    echo Install Rust from https://rustup.rs/ then reopen this folder and run this again.
    pause
    exit /b 1
)

cargo build --release

if %errorlevel% neq 0 (
    echo.
    echo Build failed.
    pause
    exit /b 1
)

mkdir dist 2>nul
copy /Y target\release\blockops_static_ip_tool.exe dist\BlockOps_Static_IP_Tool.exe

echo.
echo Done.
echo Ready-to-distribute EXE:
echo dist\BlockOps_Static_IP_Tool.exe
echo.
pause
