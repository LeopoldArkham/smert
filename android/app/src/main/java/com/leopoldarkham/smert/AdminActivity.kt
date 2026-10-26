package com.leopoldarkham.smert

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.view.WindowInsets
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.FrameLayout
import android.window.OnBackInvokedDispatcher

/**
 * smert's /admin page, full screen. It has its own launcher entry ("smert
 * admin") so that it can be picked as a lock-screen shortcut.
 */
class AdminActivity : Activity() {
    private lateinit var web: WebView

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val serverHost = Uri.parse(SERVER_URL).host

        web = WebView(this).apply {
            setBackgroundColor(getColor(R.color.background))
            settings.javaScriptEnabled = true
            settings.domStorageEnabled = true
            webViewClient = object : WebViewClient() {
                // Links off the smert server (e.g. Bible Gateway) open in the browser.
                override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
                    if (request.url.host == serverHost) return false
                    startActivity(Intent(Intent.ACTION_VIEW, request.url))
                    return true
                }
            }
        }

        // The app is drawn edge to edge: keep the page clear of the status
        // bar, the navigation bar and the keyboard.
        val root = FrameLayout(this)
        root.addView(web)
        root.setOnApplyWindowInsetsListener { view, insets ->
            val bars = insets.getInsets(WindowInsets.Type.systemBars() or WindowInsets.Type.ime())
            view.setPadding(bars.left, bars.top, bars.right, bars.bottom)
            insets
        }
        setContentView(root)
        web.loadUrl("$SERVER_URL/admin")

        onBackInvokedDispatcher.registerOnBackInvokedCallback(OnBackInvokedDispatcher.PRIORITY_DEFAULT) {
            if (web.canGoBack()) web.goBack() else finish()
        }
    }

    override fun onDestroy() {
        web.destroy()
        super.onDestroy()
    }
}
