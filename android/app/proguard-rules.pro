# UniFFI's bindings call into the Rust core through JNA, which finds its
# structures and callbacks by reflection.
-keep class com.sun.jna.** { *; }
-keep class * implements com.sun.jna.** { *; }
-keep class soy.iko.fold.core.** { *; }
-dontwarn java.awt.**
