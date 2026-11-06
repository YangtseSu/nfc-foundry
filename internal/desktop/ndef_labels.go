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
	case "wifi":
		return l.NdefTplWiFi()
	case "smartposter":
		return l.NdefTplSmartPoster()
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
	case "smartposter":
		return l.NdefKindSmartPoster()
	case "vcard":
		return l.NdefKindVCard()
	case "wifi":
		return l.NdefKindWiFi()
	case "bluetooth":
		return l.NdefKindBluetooth()
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
	default:
		return value
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
