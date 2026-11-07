package desktop

import (
	"github.com/YangtseSu/nfc-foundry/app"
	"github.com/YangtseSu/nfc-foundry/locales"
)

func ndefTemplateLabel(l locales.Messages, id string) string {
	switch id {
	case "text":
		return l.NdefTplText()
	case "uri":
		return l.NdefTplURI()
	case "custom":
		return l.NdefTplCustom()
	case "search":
		return l.NdefTplSearch()
	case "payment":
		return l.NdefTplPayment()
	case "social":
		return l.NdefTplSocial()
	case "app":
		return l.NdefTplApp()
	case "email":
		return l.NdefTplEmail()
	case "contact":
		return l.NdefTplContact()
	case "phone":
		return l.NdefTplPhone()
	case "sms":
		return l.NdefTplSMS()
	case "location":
		return l.NdefTplLocation()
	case "address":
		return l.NdefTplAddress()
	case "bluetooth":
		return l.NdefTplBluetooth()
	case "bluetooth-le":
		return l.NdefTplBluetoothLE()
	case "wifi":
		return l.NdefTplWiFi()
	case "smartposter":
		return l.NdefTplSmartPoster()
	case "dpp":
		return l.NdefTplDPP()
	default:
		return id
	}
}

func ndefFieldLabel(l locales.Messages, key string) string {
	switch key {
	case "text":
		return l.NdefFieldText()
	case "uri":
		return l.NdefFieldURI()
	case "search_prefix":
		return l.NdefFieldSearchPrefix()
	case "query":
		return l.NdefFieldQuery()
	case "package":
		return l.NdefFieldPackage()
	case "to":
		return l.NdefFieldTo()
	case "subject":
		return l.NdefFieldSubject()
	case "body":
		return l.NdefFieldBody()
	case "name":
		return l.NdefFieldName()
	case "organization":
		return l.NdefFieldOrganization()
	case "job_title":
		return l.NdefFieldJobTitle()
	case "phone":
		return l.NdefFieldPhone()
	case "number":
		return l.NdefFieldNumber()
	case "email":
		return l.NdefFieldEmail()
	case "address":
		return l.NdefFieldAddress()
	case "url":
		return l.NdefFieldURL()
	case "note":
		return l.NdefFieldNote()
	case "latitude":
		return l.NdefFieldLatitude()
	case "longitude":
		return l.NdefFieldLongitude()
	case "label":
		return l.NdefFieldLabel()
	case "ssid":
		return l.NdefFieldSSID()
	case "security":
		return l.NdefFieldSecurity()
	case "password":
		return l.NdefFieldPassword()
	case "auth":
		return l.NdefFieldAuth()
	case "encryption":
		return l.NdefFieldEncryption()
	case "network_key":
		return l.NdefFieldNetworkKey()
	case "mac":
		return l.NdefFieldMAC()
	case "bt_address":
		return l.NdefFieldBTAddress()
	case "bt_name":
		return l.NdefFieldBTName()
	case "bt_class":
		return l.NdefFieldBTClass()
	case "payload":
		return l.NdefFieldPayload()
	case "title":
		return l.NdefFieldTitle()
	case "dpp_uri":
		return l.NdefFieldDPPURI()
	case "dpp_mac":
		return l.NdefFieldDPPMAC()
	case "dpp_version":
		return l.NdefFieldDPPVersion()
	case "dpp_channels":
		return l.NdefFieldDPPChannels()
	case "dpp_key":
		return l.NdefFieldDPPKey()
	case "dpp_info":
		return l.NdefFieldDPPInfo()
	case "language":
		return l.NdefFieldLanguage()
	case "search_provider":
		return l.NdefFieldSearchProvider()
	case "social_provider":
		return l.NdefFieldSocialProvider()
	case "payment_provider":
		return l.NdefFieldPaymentProvider()
	case "handle":
		return l.NdefFieldHandle()
	case "address_type":
		return l.NdefFieldAddressType()
	case "role":
		return l.NdefFieldRole()
	case "action":
		return l.NdefFieldAction()
	case "resource_type":
		return l.NdefFieldResourceType()
	case "resource_size":
		return l.NdefFieldResourceSize()
	case "bt_address_type":
		return l.NdefFieldBTAddressType()
	case "bt_role":
		return l.NdefFieldBTRole()
	default:
		return key
	}
}

func ndefKindLabel(l locales.Messages, kind string) string {
	switch kind {
	case "text":
		return l.NdefKindText()
	case "uri":
		return l.NdefKindURI()
	case "url":
		return l.NdefKindURL()
	case "email":
		return l.NdefKindEmail()
	case "phone":
		return l.NdefKindPhone()
	case "sms":
		return l.NdefKindSMS()
	case "location":
		return l.NdefKindLocation()
	case "dpp":
		return l.NdefKindDPP()
	case "smartposter":
		return l.NdefKindSmartPoster()
	case "vcard":
		return l.NdefKindVCard()
	case "wifi":
		return l.NdefKindWiFi()
	case "bluetooth":
		return l.NdefKindBluetooth()
	case "bluetooth-le":
		return l.NdefKindBluetoothLE()
	case "app":
		return l.NdefKindApp()
	case "mime":
		return l.NdefKindMIME()
	case "external":
		return l.NdefKindExternal()
	case "absolute-uri":
		return l.NdefKindAbsoluteURI()
	case "json":
		return l.NdefKindJSON()
	default:
		return l.NdefKindUnknown()
	}
}

func ndefSecurityLabel(l locales.Messages, value string) string {
	switch value {
	case "open":
		return l.NdefSecurityOpen()
	case "wpa":
		return l.NdefSecurityWPA()
	case "wpa2":
		return l.NdefSecurityWPA2()
	case "wpa2-wpa":
		return l.NdefSecurityWPA2WPA()
	case "wpa2-wpa3":
		return l.NdefSecurityWPA3Transition()
	default:
		return value
	}
}

func ndefOptionLabel(l locales.Messages, field, option string) string {
	switch field {
	case "language":
		return option
	case "security":
		return ndefSecurityLabel(l, option)
	case "search_provider", "social_provider", "payment_provider":
		return ndefProviderLabel(l, option)
	case "address_type":
		switch option {
		case "public":
			return l.NdefAddressPublic()
		case "random":
			return l.NdefAddressRandom()
		}
	case "role":
		switch option {
		case "":
			return l.NdefRoleNone()
		case "peripheral":
			return l.NdefRolePeripheral()
		case "central":
			return l.NdefRoleCentral()
		case "both":
			return l.NdefRoleBoth()
		}
	case "action":
		switch option {
		case "":
			return l.NdefActionNone()
		case "do":
			return l.NdefActionDo()
		case "save":
			return l.NdefActionSave()
		case "edit":
			return l.NdefActionOpen()
		}
	}
	return option
}

func ndefProviderLabel(l locales.Messages, id string) string {
	switch id {
	case "duckduckgo":
		return l.NdefProviderDuckDuckGo()
	case "google":
		return l.NdefProviderGoogle()
	case "bing":
		return l.NdefProviderBing()
	case "brave":
		return l.NdefProviderBrave()
	case "startpage":
		return l.NdefProviderStartpage()
	case "wikipedia":
		return l.NdefProviderWikipedia()
	case "github":
		return l.NdefProviderGitHub()
	case "x":
		return l.NdefProviderX()
	case "mastodon":
		return l.NdefProviderMastodon()
	case "linkedin":
		return l.NdefProviderLinkedIn()
	case "instagram":
		return l.NdefProviderInstagram()
	case "youtube":
		return l.NdefProviderYouTube()
	case "facebook":
		return l.NdefProviderFacebook()
	case "telegram":
		return l.NdefProviderTelegram()
	case "paypal":
		return l.NdefProviderPayPal()
	case "venmo":
		return l.NdefProviderVenmo()
	case "cashapp":
		return l.NdefProviderCashApp()
	case "revolut":
		return l.NdefProviderRevolut()
	case "kofi":
		return l.NdefProviderKofi()
	case "buymeacoffee":
		return l.NdefProviderBuyMeACoffee()
	default:
		return id
	}
}

func ndefTemplateByID(templates []app.Type2NDEFTemplateDTO, id string) (app.Type2NDEFTemplateDTO, bool) {
	for _, template := range templates {
		if template.ID == id {
			return template, true
		}
	}
	return app.Type2NDEFTemplateDTO{}, false
}
