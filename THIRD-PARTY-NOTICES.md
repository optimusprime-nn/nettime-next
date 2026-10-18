# Bundled runtime components

The Windows installer bundles Python and its Tcl/Tk and SQLite components,
pywin32, and the PyInstaller bootloader. Their licenses remain applicable.
PyInstaller's distribution exception permits distributing the frozen programs;
the application is not a modification of PyInstaller.

- [Python license](https://docs.python.org/3/license.html)
- [pywin32 license](https://github.com/mhammond/pywin32/blob/main/Pythonwin/License.txt)
- [PyInstaller license and exception](https://pyinstaller.org/en/stable/license.html)
- [Tcl/Tk license](https://www.tcl-lang.org/software/tcltk/license.html)
- [SQLite public domain notice](https://www.sqlite.org/copyright.html)

License files from the build environment are copied into the installer payload
by the packaging script. Inno Setup produces the installer; its compiler is a
build dependency and is not installed on the end user's machine.

The NTS client also bundles pyOpenSSL (Apache-2.0), cryptography and its OpenSSL
runtime (Apache-2.0/BSD notices as distributed), service-identity (MIT), certifi
(MPL-2.0 CA bundle), cffi, pycparser, attrs and typing-extensions. Their distributed
license files are copied into ThirdPartyLicenses. No cryptographic primitives
are implemented by this application.
