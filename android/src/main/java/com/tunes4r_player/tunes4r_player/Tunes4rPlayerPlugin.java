package com.tunes4r_player.tunes4r_player;

import android.content.Context;
import androidx.annotation.NonNull;
import io.flutter.embedding.engine.plugins.FlutterPlugin;

public class Tunes4rPlayerPlugin implements FlutterPlugin {

    static {
        System.loadLibrary("tunes4r");
    }

    private static native void nativeInit(Context context);

    @Override
    public void onAttachedToEngine(@NonNull FlutterPluginBinding binding) {
        nativeInit(binding.getApplicationContext());
    }

    @Override
    public void onDetachedFromEngine(@NonNull FlutterPluginBinding binding) {
    }
}
