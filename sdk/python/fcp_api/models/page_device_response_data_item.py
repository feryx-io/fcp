from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.firmware_status import FirmwareStatus
from ..types import UNSET, Unset
from typing import cast
import datetime

if TYPE_CHECKING:
  from ..models.device_line import DeviceLine
  from ..models.e911_address_response import E911AddressResponse
  from ..models.line_key import LineKey





T = TypeVar("T", bound="PageDeviceResponseDataItem")



@_attrs_define
class PageDeviceResponseDataItem:
    """ Device response.

        Attributes:
            id (str):
            name (str):
            device_type (str):
            tenant_id (str):
            has_sip_credentials (bool):
            hoteling_host (bool): A shared phone users may log in on.
            registration_state (str):
            max_lines (int):
            supports_video (bool):
            supports_blf (bool):
            codec_support (list[str]):
            line_keys (list[LineKey]): The keys programmed on the phone, in order.
            lines (list[DeviceLine]): The lines it registers and takes calls on (`docs/DEVICE_LINES.md`);
                empty, its own line (and a phone's shared line) as before lines.
            has_provisioning_token (bool):
            enabled (bool):
            created_at (datetime.datetime):
            updated_at (datetime.datetime):
            mac_address (None | str | Unset):
            model (None | str | Unset):
            vendor (None | str | Unset): `yealink`, `polycom`, `grandstream`, `cisco`, `fanvil` or `snom`:
                with the model, it picks the template when the device names none.
            firmware_version (None | str | Unset):
            firmware_target_version (None | str | Unset): The version its tenant's firmware target points it at
                (`docs/PROVISIONING.md` §11), and whether it runs it: `current`,
                `behind`, `unknown` (it has not said). Unset without a target.
            firmware_status (FirmwareStatus | None | Unset):
            owner_user_id (None | str | Unset):
            sip_username (None | str | Unset): The username of the device's own credentials; the password and its
                hash are never returned (`docs/ENTERPRISE_CALLING.md` §3.8).
            sip_uri (None | str | Unset):
            last_registration (datetime.datetime | None | Unset):
            last_ip_address (None | str | Unset):
            user_agent (None | str | Unset):
            push_provider (None | str | Unset): The provider that wakes it when it sleeps (`apns`, `fcm`), while it
                is registered with a push address (`docs/PUSH.md` §2.1).
            location (None | str | Unset):
            location_id (None | str | Unset):
            e911_override (E911AddressResponse | None | Unset):
            e911_trust_location (bool | None | Unset): Whether this device is trusted to send its own location with a
                call (`docs/E911_BLOCK_2.md` §6.3); `null` follows its type.
            provisioning_url (None | str | Unset):
            config_template_id (None | str | Unset):
            provisioning_token (None | str | Unset):
     """

    id: str
    name: str
    device_type: str
    tenant_id: str
    has_sip_credentials: bool
    hoteling_host: bool
    registration_state: str
    max_lines: int
    supports_video: bool
    supports_blf: bool
    codec_support: list[str]
    line_keys: list[LineKey]
    lines: list[DeviceLine]
    has_provisioning_token: bool
    enabled: bool
    created_at: datetime.datetime
    updated_at: datetime.datetime
    mac_address: None | str | Unset = UNSET
    model: None | str | Unset = UNSET
    vendor: None | str | Unset = UNSET
    firmware_version: None | str | Unset = UNSET
    firmware_target_version: None | str | Unset = UNSET
    firmware_status: FirmwareStatus | None | Unset = UNSET
    owner_user_id: None | str | Unset = UNSET
    sip_username: None | str | Unset = UNSET
    sip_uri: None | str | Unset = UNSET
    last_registration: datetime.datetime | None | Unset = UNSET
    last_ip_address: None | str | Unset = UNSET
    user_agent: None | str | Unset = UNSET
    push_provider: None | str | Unset = UNSET
    location: None | str | Unset = UNSET
    location_id: None | str | Unset = UNSET
    e911_override: E911AddressResponse | None | Unset = UNSET
    e911_trust_location: bool | None | Unset = UNSET
    provisioning_url: None | str | Unset = UNSET
    config_template_id: None | str | Unset = UNSET
    provisioning_token: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.device_line import DeviceLine # noqa: PLC0415
        from ..models.e911_address_response import E911AddressResponse # noqa: PLC0415
        from ..models.line_key import LineKey # noqa: PLC0415
        id = self.id

        name = self.name

        device_type = self.device_type

        tenant_id = self.tenant_id

        has_sip_credentials = self.has_sip_credentials

        hoteling_host = self.hoteling_host

        registration_state = self.registration_state

        max_lines = self.max_lines

        supports_video = self.supports_video

        supports_blf = self.supports_blf

        codec_support = self.codec_support



        line_keys = []
        for line_keys_item_data in self.line_keys:
            line_keys_item = line_keys_item_data.to_dict()
            line_keys.append(line_keys_item)



        lines = []
        for lines_item_data in self.lines:
            lines_item = lines_item_data.to_dict()
            lines.append(lines_item)



        has_provisioning_token = self.has_provisioning_token

        enabled = self.enabled

        created_at = self.created_at.isoformat()

        updated_at = self.updated_at.isoformat()

        mac_address: None | str | Unset
        if isinstance(self.mac_address, Unset):
            mac_address = UNSET
        else:
            mac_address = self.mac_address

        model: None | str | Unset
        if isinstance(self.model, Unset):
            model = UNSET
        else:
            model = self.model

        vendor: None | str | Unset
        if isinstance(self.vendor, Unset):
            vendor = UNSET
        else:
            vendor = self.vendor

        firmware_version: None | str | Unset
        if isinstance(self.firmware_version, Unset):
            firmware_version = UNSET
        else:
            firmware_version = self.firmware_version

        firmware_target_version: None | str | Unset
        if isinstance(self.firmware_target_version, Unset):
            firmware_target_version = UNSET
        else:
            firmware_target_version = self.firmware_target_version

        firmware_status: None | str | Unset
        if isinstance(self.firmware_status, Unset):
            firmware_status = UNSET
        elif isinstance(self.firmware_status, FirmwareStatus):
            firmware_status = self.firmware_status.value
        else:
            firmware_status = self.firmware_status

        owner_user_id: None | str | Unset
        if isinstance(self.owner_user_id, Unset):
            owner_user_id = UNSET
        else:
            owner_user_id = self.owner_user_id

        sip_username: None | str | Unset
        if isinstance(self.sip_username, Unset):
            sip_username = UNSET
        else:
            sip_username = self.sip_username

        sip_uri: None | str | Unset
        if isinstance(self.sip_uri, Unset):
            sip_uri = UNSET
        else:
            sip_uri = self.sip_uri

        last_registration: None | str | Unset
        if isinstance(self.last_registration, Unset):
            last_registration = UNSET
        elif isinstance(self.last_registration, datetime.datetime):
            last_registration = self.last_registration.isoformat()
        else:
            last_registration = self.last_registration

        last_ip_address: None | str | Unset
        if isinstance(self.last_ip_address, Unset):
            last_ip_address = UNSET
        else:
            last_ip_address = self.last_ip_address

        user_agent: None | str | Unset
        if isinstance(self.user_agent, Unset):
            user_agent = UNSET
        else:
            user_agent = self.user_agent

        push_provider: None | str | Unset
        if isinstance(self.push_provider, Unset):
            push_provider = UNSET
        else:
            push_provider = self.push_provider

        location: None | str | Unset
        if isinstance(self.location, Unset):
            location = UNSET
        else:
            location = self.location

        location_id: None | str | Unset
        if isinstance(self.location_id, Unset):
            location_id = UNSET
        else:
            location_id = self.location_id

        e911_override: dict[str, Any] | None | Unset
        if isinstance(self.e911_override, Unset):
            e911_override = UNSET
        elif isinstance(self.e911_override, E911AddressResponse):
            e911_override = self.e911_override.to_dict()
        else:
            e911_override = self.e911_override

        e911_trust_location: bool | None | Unset
        if isinstance(self.e911_trust_location, Unset):
            e911_trust_location = UNSET
        else:
            e911_trust_location = self.e911_trust_location

        provisioning_url: None | str | Unset
        if isinstance(self.provisioning_url, Unset):
            provisioning_url = UNSET
        else:
            provisioning_url = self.provisioning_url

        config_template_id: None | str | Unset
        if isinstance(self.config_template_id, Unset):
            config_template_id = UNSET
        else:
            config_template_id = self.config_template_id

        provisioning_token: None | str | Unset
        if isinstance(self.provisioning_token, Unset):
            provisioning_token = UNSET
        else:
            provisioning_token = self.provisioning_token


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "name": name,
            "device_type": device_type,
            "tenant_id": tenant_id,
            "has_sip_credentials": has_sip_credentials,
            "hoteling_host": hoteling_host,
            "registration_state": registration_state,
            "max_lines": max_lines,
            "supports_video": supports_video,
            "supports_blf": supports_blf,
            "codec_support": codec_support,
            "line_keys": line_keys,
            "lines": lines,
            "has_provisioning_token": has_provisioning_token,
            "enabled": enabled,
            "created_at": created_at,
            "updated_at": updated_at,
        })
        if mac_address is not UNSET:
            field_dict["mac_address"] = mac_address
        if model is not UNSET:
            field_dict["model"] = model
        if vendor is not UNSET:
            field_dict["vendor"] = vendor
        if firmware_version is not UNSET:
            field_dict["firmware_version"] = firmware_version
        if firmware_target_version is not UNSET:
            field_dict["firmware_target_version"] = firmware_target_version
        if firmware_status is not UNSET:
            field_dict["firmware_status"] = firmware_status
        if owner_user_id is not UNSET:
            field_dict["owner_user_id"] = owner_user_id
        if sip_username is not UNSET:
            field_dict["sip_username"] = sip_username
        if sip_uri is not UNSET:
            field_dict["sip_uri"] = sip_uri
        if last_registration is not UNSET:
            field_dict["last_registration"] = last_registration
        if last_ip_address is not UNSET:
            field_dict["last_ip_address"] = last_ip_address
        if user_agent is not UNSET:
            field_dict["user_agent"] = user_agent
        if push_provider is not UNSET:
            field_dict["push_provider"] = push_provider
        if location is not UNSET:
            field_dict["location"] = location
        if location_id is not UNSET:
            field_dict["location_id"] = location_id
        if e911_override is not UNSET:
            field_dict["e911_override"] = e911_override
        if e911_trust_location is not UNSET:
            field_dict["e911_trust_location"] = e911_trust_location
        if provisioning_url is not UNSET:
            field_dict["provisioning_url"] = provisioning_url
        if config_template_id is not UNSET:
            field_dict["config_template_id"] = config_template_id
        if provisioning_token is not UNSET:
            field_dict["provisioning_token"] = provisioning_token

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.device_line import DeviceLine # noqa: PLC0415
        from ..models.e911_address_response import E911AddressResponse # noqa: PLC0415
        from ..models.line_key import LineKey # noqa: PLC0415
        d = dict(src_dict)
        id = d.pop("id")

        name = d.pop("name")

        device_type = d.pop("device_type")

        tenant_id = d.pop("tenant_id")

        has_sip_credentials = d.pop("has_sip_credentials")

        hoteling_host = d.pop("hoteling_host")

        registration_state = d.pop("registration_state")

        max_lines = d.pop("max_lines")

        supports_video = d.pop("supports_video")

        supports_blf = d.pop("supports_blf")

        codec_support = cast(list[str], d.pop("codec_support"))


        line_keys = []
        _line_keys = d.pop("line_keys")
        for line_keys_item_data in (_line_keys):
            line_keys_item = LineKey.from_dict(line_keys_item_data)



            line_keys.append(line_keys_item)


        lines = []
        _lines = d.pop("lines")
        for lines_item_data in (_lines):
            lines_item = DeviceLine.from_dict(lines_item_data)



            lines.append(lines_item)


        has_provisioning_token = d.pop("has_provisioning_token")

        enabled = d.pop("enabled")

        created_at = datetime.datetime.fromisoformat(d.pop("created_at"))




        updated_at = datetime.datetime.fromisoformat(d.pop("updated_at"))




        def _parse_mac_address(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        mac_address = _parse_mac_address(d.pop("mac_address", UNSET))


        def _parse_model(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        model = _parse_model(d.pop("model", UNSET))


        def _parse_vendor(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        vendor = _parse_vendor(d.pop("vendor", UNSET))


        def _parse_firmware_version(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        firmware_version = _parse_firmware_version(d.pop("firmware_version", UNSET))


        def _parse_firmware_target_version(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        firmware_target_version = _parse_firmware_target_version(d.pop("firmware_target_version", UNSET))


        def _parse_firmware_status(data: object) -> FirmwareStatus | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                firmware_status_type_0 = FirmwareStatus(data)



                return firmware_status_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(FirmwareStatus | None | Unset, data)

        firmware_status = _parse_firmware_status(d.pop("firmware_status", UNSET))


        def _parse_owner_user_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        owner_user_id = _parse_owner_user_id(d.pop("owner_user_id", UNSET))


        def _parse_sip_username(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        sip_username = _parse_sip_username(d.pop("sip_username", UNSET))


        def _parse_sip_uri(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        sip_uri = _parse_sip_uri(d.pop("sip_uri", UNSET))


        def _parse_last_registration(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                last_registration_type_0 = datetime.datetime.fromisoformat(data)



                return last_registration_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        last_registration = _parse_last_registration(d.pop("last_registration", UNSET))


        def _parse_last_ip_address(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        last_ip_address = _parse_last_ip_address(d.pop("last_ip_address", UNSET))


        def _parse_user_agent(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        user_agent = _parse_user_agent(d.pop("user_agent", UNSET))


        def _parse_push_provider(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        push_provider = _parse_push_provider(d.pop("push_provider", UNSET))


        def _parse_location(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        location = _parse_location(d.pop("location", UNSET))


        def _parse_location_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        location_id = _parse_location_id(d.pop("location_id", UNSET))


        def _parse_e911_override(data: object) -> E911AddressResponse | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                e911_override_type_0 = E911AddressResponse.from_dict(data)



                return e911_override_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(E911AddressResponse | None | Unset, data)

        e911_override = _parse_e911_override(d.pop("e911_override", UNSET))


        def _parse_e911_trust_location(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        e911_trust_location = _parse_e911_trust_location(d.pop("e911_trust_location", UNSET))


        def _parse_provisioning_url(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        provisioning_url = _parse_provisioning_url(d.pop("provisioning_url", UNSET))


        def _parse_config_template_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        config_template_id = _parse_config_template_id(d.pop("config_template_id", UNSET))


        def _parse_provisioning_token(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        provisioning_token = _parse_provisioning_token(d.pop("provisioning_token", UNSET))


        page_device_response_data_item = cls(
            id=id,
            name=name,
            device_type=device_type,
            tenant_id=tenant_id,
            has_sip_credentials=has_sip_credentials,
            hoteling_host=hoteling_host,
            registration_state=registration_state,
            max_lines=max_lines,
            supports_video=supports_video,
            supports_blf=supports_blf,
            codec_support=codec_support,
            line_keys=line_keys,
            lines=lines,
            has_provisioning_token=has_provisioning_token,
            enabled=enabled,
            created_at=created_at,
            updated_at=updated_at,
            mac_address=mac_address,
            model=model,
            vendor=vendor,
            firmware_version=firmware_version,
            firmware_target_version=firmware_target_version,
            firmware_status=firmware_status,
            owner_user_id=owner_user_id,
            sip_username=sip_username,
            sip_uri=sip_uri,
            last_registration=last_registration,
            last_ip_address=last_ip_address,
            user_agent=user_agent,
            push_provider=push_provider,
            location=location,
            location_id=location_id,
            e911_override=e911_override,
            e911_trust_location=e911_trust_location,
            provisioning_url=provisioning_url,
            config_template_id=config_template_id,
            provisioning_token=provisioning_token,
        )


        page_device_response_data_item.additional_properties = d
        return page_device_response_data_item

    @property
    def additional_keys(self) -> list[str]:
        return list(self.additional_properties.keys())

    def __getitem__(self, key: str) -> Any:
        return self.additional_properties[key]

    def __setitem__(self, key: str, value: Any) -> None:
        self.additional_properties[key] = value

    def __delitem__(self, key: str) -> None:
        del self.additional_properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.additional_properties
