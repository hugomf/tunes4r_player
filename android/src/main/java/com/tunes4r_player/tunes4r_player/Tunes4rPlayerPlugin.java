package com.tunes4r_player.tunes4r_player;

import androidx.annotation.NonNull;
import io.flutter.embedding.engine.plugins.FlutterPlugin;

public class Tunes4rPlayerPlugin implements FlutterPlugin {

    static {
        System.loadLibrary("tunes4r");
    }

    @Override
    public void onAttachedToEngine(@NonNull FlutterPluginBinding binding) {
    }

    @Override
    public void onDetachedFromEngine(@NonNull FlutterPluginBinding binding) {
    }
}
