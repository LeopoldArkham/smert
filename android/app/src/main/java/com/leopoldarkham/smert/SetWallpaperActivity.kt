package com.leopoldarkham.smert

import android.app.Activity
import android.app.WallpaperManager
import android.content.ComponentName
import android.content.Intent
import android.os.Bundle

/** Opens the system preview for this live wallpaper, with its "Set" button. */
class SetWallpaperActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val intent = Intent(WallpaperManager.ACTION_CHANGE_LIVE_WALLPAPER).putExtra(
            WallpaperManager.EXTRA_LIVE_WALLPAPER_COMPONENT,
            ComponentName(this, WordsWallpaperService::class.java),
        )
        startActivity(intent)
        finish()
    }
}
