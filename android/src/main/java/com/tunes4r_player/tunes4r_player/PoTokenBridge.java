package com.tunes4r_player.tunes4r_player;

import android.content.Context;
import android.os.Handler;
import android.os.Looper;
import android.util.Base64;
import android.util.Log;
import android.webkit.JavascriptInterface;
import android.webkit.WebView;
import android.webkit.WebViewClient;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.BufferedReader;
import java.io.InputStream;
import java.io.InputStreamReader;
import java.io.OutputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;

public class PoTokenBridge {
    private static final String TAG = "PoTokenBridge";
    private static final String REQUEST_KEY = "O43z0dpjhgX20SCx4KAo";
    private static final String GOOG_API_KEY = "AIzaSyDyT5W0Jh49F30Pqqtyfdf7pDLFKLJoAnw";
    private static final String CREATE_URL = "https://www.youtube.com/api/jnn/v1/Create?key=" + GOOG_API_KEY;
    private static final String GENERATE_URL = "https://www.youtube.com/api/jnn/v1/GenerateIT?key=" + GOOG_API_KEY;

    private static Context appCtx;
    private static WebView webView;
    private static String bundleJs;
    private static volatile boolean ready = false;
    private static final Object lock = new Object();
    private static final Handler mainHandler = new Handler(Looper.getMainLooper());

    public static void init(Context ctx) {
        appCtx = ctx.getApplicationContext();
    }

    private static void ensureWebView() throws Exception {
        if (ready && webView != null) return;
        synchronized (lock) {
            if (ready && webView != null) return;
            if (appCtx == null) throw new IllegalStateException("PoTokenBridge not inited");
            if (bundleJs == null) {
                try (InputStream is = appCtx.getAssets().open("bundle.cjs")) {
                    BufferedReader br = new BufferedReader(new InputStreamReader(is, StandardCharsets.UTF_8));
                    StringBuilder sb = new StringBuilder();
                    String line;
                    while ((line = br.readLine()) != null) sb.append(line).append('\n');
                    bundleJs = sb.toString();
                }
            }
            CountDownLatch latch = new CountDownLatch(1);
            AtomicReference<Exception> err = new AtomicReference<>();
            mainHandler.post(() -> {
                try {
                    webView = new WebView(appCtx);
                    webView.getSettings().setJavaScriptEnabled(true);
                    webView.getSettings().setDomStorageEnabled(true);
                    IpcBridge bridge = new IpcBridge();
                    webView.addJavascriptInterface(bridge, "ipc");
                    webView.setWebViewClient(new WebViewClient() {
                        @Override public void onPageFinished(WebView v, String url) {
                            String init = "(function(){"
                                    + "let module={exports:{}};let exports=module.exports;"
                                    + bundleJs + "\n"
                                    + "globalThis.BG=module.exports.BG||module.exports;"
                                    + "globalThis.__mintFlow=async(js,prog,gname,content)=>{"
                                    + "new Function(js)();"
                                    + "const bg=await globalThis.BG.BotGuardClient.create({program:prog,globalName:gname,globalObj:globalThis});"
                                    + "const wpo=[];const resp=await bg.snapshot({webPoSignalOutput:wpo});"
                                    + "globalThis.__wpo=wpo;return resp;};"
                                    + "globalThis.__doMint=async(it,visitor)=>{"
                                    + "const minter=await globalThis.BG.WebPoMinter.create({integrityToken:it},globalThis.__wpo);"
                                    + "return await minter.mintAsWebsafeString(visitor);};"
                                    + "})();";
                            v.evaluateJavascript(init, vv -> latch.countDown());
                        }
                    });
                    webView.loadDataWithBaseURL("https://www.youtube.com/", "<html><body></body></html>", "text/html", "utf-8", null);
                } catch (Exception e) { err.set(e); latch.countDown(); }
            });
            if (!latch.await(10, TimeUnit.SECONDS)) throw new RuntimeException("WebView init timeout");
            if (err.get() != null) throw err.get();
            ready = true;
            Thread.sleep(200);
        }
    }

    private static class IpcBridge {
        volatile String msg;
        CountDownLatch latch;
        @JavascriptInterface public void postMessage(String m) { msg = m; if (latch != null) latch.countDown(); }
    }

    private static String evalAsync(String jsWithPost) throws Exception {
        CountDownLatch latch = new CountDownLatch(1);
        IpcBridge bridge = new IpcBridge();
        bridge.latch = latch;
        mainHandler.post(() -> {
            try {
                WebView wv = webView;
                wv.removeJavascriptInterface("ipc");
                wv.addJavascriptInterface(bridge, "ipc");
                wv.evaluateJavascript(jsWithPost, null);
            } catch (Exception e) { bridge.msg = "ERR:" + e.getMessage(); latch.countDown(); }
        });
        if (!latch.await(15, TimeUnit.SECONDS)) throw new RuntimeException("eval timeout: " + jsWithPost.substring(0, Math.min(120, jsWithPost.length())));
        if (bridge.msg == null) throw new RuntimeException("no ipc message");
        return bridge.msg;
    }

    private static String jsQuote(String s) { return JSONObject.quote(s); }

    private static String httpPost(String urlStr, String body) throws Exception {
        URL url = new URL(urlStr);
        HttpURLConnection c = (HttpURLConnection) url.openConnection();
        c.setRequestMethod("POST");
        c.setConnectTimeout(10000); c.setReadTimeout(10000);
        c.setDoOutput(true);
        c.setRequestProperty("Content-Type", "application/json+protobuf");
        c.setRequestProperty("x-goog-api-key", GOOG_API_KEY);
        c.setRequestProperty("x-user-agent", "grpc-web-javascript/0.1");
        try (OutputStream os = c.getOutputStream()) { os.write(body.getBytes(StandardCharsets.UTF_8)); }
        int code = c.getResponseCode();
        InputStream is = code >= 200 && code < 300 ? c.getInputStream() : c.getErrorStream();
        BufferedReader br = new BufferedReader(new InputStreamReader(is, StandardCharsets.UTF_8));
        StringBuilder sb = new StringBuilder(); String line;
        while ((line = br.readLine()) != null) sb.append(line);
        if (code < 200 || code >= 300) throw new RuntimeException("HTTP " + code + ": " + sb);
        return sb.toString();
    }

    private static String httpGet(String urlStr) throws Exception {
        URL url = new URL(urlStr);
        HttpURLConnection c = (HttpURLConnection) url.openConnection();
        c.setConnectTimeout(10000); c.setReadTimeout(10000);
        BufferedReader br = new BufferedReader(new InputStreamReader(c.getInputStream(), StandardCharsets.UTF_8));
        StringBuilder sb = new StringBuilder(); String line;
        while ((line = br.readLine()) != null) sb.append(line).append('\n');
        return sb.toString();
    }

    public static String mintSync(String visitorData) throws Exception {
        synchronized (lock) { return mintInternal(visitorData); }
    }

    private static String mintInternal(String visitorData) throws Exception {
        ensureWebView();
        String createResp = httpPost(CREATE_URL, "[\"" + REQUEST_KEY + "\"]");
        JSONArray outer = new JSONArray(createResp);
        String b64 = null;
        if (outer.length() > 1 && outer.get(1) instanceof String) b64 = outer.getString(1);
        JSONArray cdata;
        if (b64 != null && !b64.isEmpty()) {
            byte[] decoded = Base64.decode(b64, Base64.DEFAULT);
            byte[] descr = new byte[decoded.length];
            for (int i = 0; i < decoded.length; i++) descr[i] = (byte) ((decoded[i] + 97) & 0xFF);
            cdata = new JSONArray(new String(descr, StandardCharsets.UTF_8));
        } else {
            cdata = outer.getJSONArray(0);
        }
        if (cdata.length() < 6) throw new RuntimeException("cdata len " + cdata.length());
        String prog = cdata.getString(4);
        String gname = cdata.getString(5);
        String interpJs = null; String interpUrl = null;
        if (!cdata.isNull(1) && cdata.get(1) instanceof JSONArray) {
            JSONArray a = cdata.getJSONArray(1);
            for (int i = 0; i < a.length(); i++) { String s = a.optString(i, ""); if (!s.isEmpty()) { interpJs = s; break; } }
        }
        if (!cdata.isNull(2) && cdata.get(2) instanceof JSONArray) {
            JSONArray a = cdata.getJSONArray(2);
            for (int i = 0; i < a.length(); i++) { String s = a.optString(i, ""); if (!s.isEmpty()) { interpUrl = s; break; } }
        }
        String js;
        if (interpJs != null && !interpJs.isEmpty()) js = interpJs;
        else if (interpUrl != null && !interpUrl.isEmpty()) {
            String full = interpUrl.startsWith("//") ? "https:" + interpUrl : interpUrl;
            js = httpGet(full);
        } else throw new RuntimeException("no interpreter");
        String bgExpr = "globalThis.__mintFlow(" + jsQuote(js) + "," + jsQuote(prog) + "," + jsQuote(gname) + "," + jsQuote(visitorData) + ").then(r=>ipc.postMessage('BG:'+r)).catch(e=>ipc.postMessage('BGERR:'+e.message))";
        String bgMsg = evalAsync(bgExpr);
        if (bgMsg.startsWith("BGERR:")) throw new RuntimeException(bgMsg);
        if (!bgMsg.startsWith("BG:")) throw new RuntimeException("bad bg msg: " + bgMsg);
        String bgResp = bgMsg.substring(3);
        JSONArray genBody = new JSONArray(); genBody.put(REQUEST_KEY); genBody.put(bgResp);
        String genResp = httpPost(GENERATE_URL, genBody.toString());
        JSONArray genArr = new JSONArray(genResp);
        String it = genArr.getString(0);
        String mintExpr = "globalThis.__doMint(" + jsQuote(it) + "," + jsQuote(visitorData) + ").then(p=>ipc.postMessage('POT:'+p)).catch(e=>ipc.postMessage('POTERR:'+e.message))";
        String potMsg = evalAsync(mintExpr);
        if (potMsg.startsWith("POTERR:")) throw new RuntimeException(potMsg);
        if (!potMsg.startsWith("POT:")) throw new RuntimeException("bad pot msg: " + potMsg);
        return potMsg.substring(4);
    }
}
