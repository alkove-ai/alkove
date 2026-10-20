# the native library calls these by name
-keepclasseswithmembernames class ai.alkove.Native { native <methods>; }
-keep class ai.alkove.AlkoveException { <init>(java.lang.String); }
