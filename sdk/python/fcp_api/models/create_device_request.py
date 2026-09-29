from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.e911_address_response import E911AddressResponse





T = TypeVar("T", bound="CreateDeviceRequest")



@_attrs_define
class CreateDeviceRequest:
    """ Create device request.

        Attributes:
            name (str):
            device_type (str):
            tenant_id (str | Unset): A TenantId: `tenant-<uuid>`, or the bare UUID
            owner_user_id (None | str | Unset):
            sip_username (None | str | Unset): Credentials of the device's own (§3.8): both, or neither.
            sip_password (None | str | Unset):
            hoteling_host (bool | None | Unset): A shared phone users may log in on.
            mac_address (None | str | Unset):
            model (None | str | Unset):
            sip_uri (None | str | Unset):
            max_lines (int | None | Unset):
            supports_video (bool | None | Unset):
            supports_blf (bool | None | Unset):
            location (None | str | Unset):
            location_id (None | str | Unset):
            e911_override (E911AddressResponse | None | Unset):
            e911_trust_location (bool | None | Unset): Trust this device's own location (`docs/E911_BLOCK_2.md` §6.3).
            provisioning_token (None | str | Unset): Optional per-device provisioning token. If omitted for a device with a
                MAC address, a token is generated and returned once in the create response.
     """

    name: str
    device_type: str
    tenant_id: str | Unset = UNSET
    owner_user_id: None | str | Unset = UNSET
    sip_username: None | str | Unset = UNSET
    sip_password: None | str | Unset = UNSET
    hoteling_host: bool | None | Unset = UNSET
    mac_address: None | str | Unset = UNSET
    model: None | str | Unset = UNSET
    sip_uri: None | str | Unset = UNSET
    max_lines: int | None | Unset = UNSET
    supports_video: bool | None | Unset = UNSET
    supports_blf: bool | None | Unset = UNSET
    location: None | str | Unset = UNSET
    location_id: None | str | Unset = UNSET
    e911_override: E911AddressResponse | None | Unset = UNSET
    e911_trust_location: bool | None | Unset = UNSET
    provisioning_token: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.e911_address_response import E911AddressResponse # noqa: PLC0415
        name = self.name

        device_type = self.device_type

        tenant_id = self.tenant_id

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

        sip_password: None | str | Unset
        if isinstance(self.sip_password, Unset):
            sip_password = UNSET
        else:
            sip_password = self.sip_password

        hoteling_host: bool | None | Unset
        if isinstance(self.hoteling_host, Unset):
            hoteling_host = UNSET
        else:
            hoteling_host = self.hoteling_host

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

        sip_uri: None | str | Unset
        if isinstance(self.sip_uri, Unset):
            sip_uri = UNSET
        else:
            sip_uri = self.sip_uri

        max_lines: int | None | Unset
        if isinstance(self.max_lines, Unset):
            max_lines = UNSET
        else:
            max_lines = self.max_lines

        supports_video: bool | None | Unset
        if isinstance(self.supports_video, Unset):
            supports_video = UNSET
        else:
            supports_video = self.supports_video

        supports_blf: bool | None | Unset
        if isinstance(self.supports_blf, Unset):
            supports_blf = UNSET
        else:
            supports_blf = self.supports_blf

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

        provisioning_token: None | str | Unset
        if isinstance(self.provisioning_token, Unset):
            provisioning_token = UNSET
        else:
            provisioning_token = self.provisioning_token


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "name": name,
            "device_type": device_type,
        })
        if tenant_id is not UNSET:
            field_dict["tenant_id"] = tenant_id
        if owner_user_id is not UNSET:
            field_dict["owner_user_id"] = owner_user_id
        if sip_username is not UNSET:
            field_dict["sip_username"] = sip_username
        if sip_password is not UNSET:
            field_dict["sip_password"] = sip_password
        if hoteling_host is not UNSET:
            field_dict["hoteling_host"] = hoteling_host
        if mac_address is not UNSET:
            field_dict["mac_address"] = mac_address
        if model is not UNSET:
            field_dict["model"] = model
        if sip_uri is not UNSET:
            field_dict["sip_uri"] = sip_uri
        if max_lines is not UNSET:
            field_dict["max_lines"] = max_lines
        if supports_video is not UNSET:
            field_dict["supports_video"] = supports_video
        if supports_blf is not UNSET:
            field_dict["supports_blf"] = supports_blf
        if location is not UNSET:
            field_dict["location"] = location
        if location_id is not UNSET:
            field_dict["location_id"] = location_id
        if e911_override is not UNSET:
            field_dict["e911_override"] = e911_override
        if e911_trust_location is not UNSET:
            field_dict["e911_trust_location"] = e911_trust_location
        if provisioning_token is not UNSET:
            field_dict["provisioning_token"] = provisioning_token

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.e911_address_response import E911AddressResponse # noqa: PLC0415
        d = dict(src_dict)
        name = d.pop("name")

        device_type = d.pop("device_type")

        tenant_id = d.pop("tenant_id", UNSET)

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


        def _parse_sip_password(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        sip_password = _parse_sip_password(d.pop("sip_password", UNSET))


        def _parse_hoteling_host(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        hoteling_host = _parse_hoteling_host(d.pop("hoteling_host", UNSET))


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


        def _parse_sip_uri(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        sip_uri = _parse_sip_uri(d.pop("sip_uri", UNSET))


        def _parse_max_lines(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        max_lines = _parse_max_lines(d.pop("max_lines", UNSET))


        def _parse_supports_video(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        supports_video = _parse_supports_video(d.pop("supports_video", UNSET))


        def _parse_supports_blf(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        supports_blf = _parse_supports_blf(d.pop("supports_blf", UNSET))


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


        def _parse_provisioning_token(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        provisioning_token = _parse_provisioning_token(d.pop("provisioning_token", UNSET))


        create_device_request = cls(
            name=name,
            device_type=device_type,
            tenant_id=tenant_id,
            owner_user_id=owner_user_id,
            sip_username=sip_username,
            sip_password=sip_password,
            hoteling_host=hoteling_host,
            mac_address=mac_address,
            model=model,
            sip_uri=sip_uri,
            max_lines=max_lines,
            supports_video=supports_video,
            supports_blf=supports_blf,
            location=location,
            location_id=location_id,
            e911_override=e911_override,
            e911_trust_location=e911_trust_location,
            provisioning_token=provisioning_token,
        )


        create_device_request.additional_properties = d
        return create_device_request

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
