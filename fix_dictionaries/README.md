
# FIX Dictionaries

This directory will contain FIX XML dictionary files for code generation.

Currently supported:
- Basic admin messages (Logon, Heartbeat, TestRequest, Logout, ResendRequest, SequenceReset)

Future plans:
- Parse FIX 4.2, 4.4, FIXT 1.1, and FIX 5.0SP2 XML dictionaries
- Generate complete message types with validation
- Support custom fields and components

## Usage

Place FIX XML dictionary files in this directory. The build script will automatically parse them and generate corresponding Rust types.

Example dictionary structure:
```
fix_dictionaries/
├── FIX42.xml
├── FIX44.xml
├── FIXT11.xml
└── FIX50SP2.xml
```
