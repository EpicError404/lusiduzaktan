# 🛡️ Lusid - Uzak Masaüstü Yönetici Kontrolü (Remote Desktop)

Windows 10 ve Windows 11 uyumlu, TeamViewer benzeri, kullanıcı onay mekanizmalı ve tam yönetici (Administrator) yetkili uzaktan masaüstü kontrol yazılımı.

---

## ✨ Özellikler

* **🌐 Cloudflare TLS 1.3 Güvenli İnternet Tüneli:** Modem/router port yönlendirme gerekmez! Türk Telekom, Turkcell, Vodafone dahil tüm İSS sansür/güvenlik duvarlarını ve CGNAT engellerini Cloudflare'in uçtan uca şifreli TLS 1.3 ağıyla aşar.
* **⚡ Ultra Düşük Gecikme:** Donanım hızlandırmalı DWM/DirectX ekran yakalama ve LZ4 gerçek zamanlı sıkıştırma.
* **🛡️ Tam Yönetici Yetkisi:** Windows `SendInput` API'si ile kilitli pencereleri ve UAC pencerelerini tam yetkiyle kontrol edebilme.
* **🔒 İki Kademeli Güvenlik:** 
  1. 6 haneli rastgele tek kullanımlık şifre doğrulaması.
  2. Müşteri ekranında **"✅ Bağlantıyı Kabul Et"** pop-up onayı (kullanıcı tıklamadan ekran görüntüsü veya fare kontrolü asla aktarılmaz).
* **🎨 Cyberpunk / Synthwave UI:** Parlayan neon çerçeveler, koyu siber tema ve özel Lusid logosu.
* **📦 Bağımsız (Standalone):** Kurulum gerektirmez, doğrudan çalışır.

---

## 🚀 Nasıl Kullanılır?

### 1. Müşteri (Kontrol Edilecek Bilgisayar):
1. `lusid.exe` dosyasını sağ tıklayıp **"Yönetici Olarak Çalıştır"** deyin.
2. Sol panelde beliren **Cihaz ID** (9 haneli numara, örn: `742 819 305`) ve **Tek Kullanımlık Şifre** bilgisini teknisyene iletin.
3. Teknisyen bağlandığında ekranda çıkan pencereden **"✅ Bağlantıyı Kabul Et"** butonuna basın.

### 2. Teknisyen (Kontrol Eden Kişi):
1. `lusid.exe` dosyasını açın.
2. Sağ taraftaki **"Uzak Cihazı Kontrol Et"** paneline müşterinin verdiği 9 haneli Cihaz ID'sini (örn: `742 819 305` veya boşluksuz `742819305`) ve Şifreyi girin. *(Yerel Ağda doğrudan `192.168.1.50:8990` da yazabilirsiniz)*.
3. **"⚡ BAĞLAN (Admin Kontrolü)"** butonuna tıklayın.
4. Müşteri onayladığında canlı masaüstü penceresi anında açılacaktır.

---

## 📁 Dosyalar

* `lusid.exe` - Ana çalıştırılabilir bağımsız uygulama.
* `cloudflared.exe` - Güvenli TLS 1.3 tünelini sağlayan Cloudflare bileşeni (aynı klasörde bulunması önerilir, yoksa Lusid otomatik indirir).
* `lusid.exe.manifest` - Windows 10/11 için yönetici yetki ve uyumluluk bildirim dosyası.
* `Lusid-v0.1.0-Windows-x64.zip` - Tüm gerekli dosyaları içeren hazır paket.
