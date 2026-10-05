from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="DeviceAppResponse")



@_attrs_define
class DeviceAppResponse:
    """ An app install's session, on its device.

        Attributes:
            install_id (str):
            signed_in (bool): Whether the app is signed in now.
            platform (None | str | Unset):
            version (None | str | Unset):
            signed_in_at (datetime.datetime | None | Unset): When it signed in, while it is.
            last_seen (datetime.datetime | None | Unset): When it last refreshed its session, while it is signed in.
     """

    install_id: str
    signed_in: bool
    platform: None | str | Unset = UNSET
    version: None | str | Unset = UNSET
    signed_in_at: datetime.datetime | None | Unset = UNSET
    last_seen: datetime.datetime | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        install_id = self.install_id

        signed_in = self.signed_in

        platform: None | str | Unset
        if isinstance(self.platform, Unset):
            platform = UNSET
        else:
            platform = self.platform

        version: None | str | Unset
        if isinstance(self.version, Unset):
            version = UNSET
        else:
            version = self.version

        signed_in_at: None | str | Unset
        if isinstance(self.signed_in_at, Unset):
            signed_in_at = UNSET
        elif isinstance(self.signed_in_at, datetime.datetime):
            signed_in_at = self.signed_in_at.isoformat()
        else:
            signed_in_at = self.signed_in_at

        last_seen: None | str | Unset
        if isinstance(self.last_seen, Unset):
            last_seen = UNSET
        elif isinstance(self.last_seen, datetime.datetime):
            last_seen = self.last_seen.isoformat()
        else:
            last_seen = self.last_seen


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "install_id": install_id,
            "signed_in": signed_in,
        })
        if platform is not UNSET:
            field_dict["platform"] = platform
        if version is not UNSET:
            field_dict["version"] = version
        if signed_in_at is not UNSET:
            field_dict["signed_in_at"] = signed_in_at
        if last_seen is not UNSET:
            field_dict["last_seen"] = last_seen

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        install_id = d.pop("install_id")

        signed_in = d.pop("signed_in")

        def _parse_platform(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        platform = _parse_platform(d.pop("platform", UNSET))


        def _parse_version(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        version = _parse_version(d.pop("version", UNSET))


        def _parse_signed_in_at(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                signed_in_at_type_0 = datetime.datetime.fromisoformat(data)



                return signed_in_at_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        signed_in_at = _parse_signed_in_at(d.pop("signed_in_at", UNSET))


        def _parse_last_seen(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                last_seen_type_0 = datetime.datetime.fromisoformat(data)



                return last_seen_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        last_seen = _parse_last_seen(d.pop("last_seen", UNSET))


        device_app_response = cls(
            install_id=install_id,
            signed_in=signed_in,
            platform=platform,
            version=version,
            signed_in_at=signed_in_at,
            last_seen=last_seen,
        )


        device_app_response.additional_properties = d
        return device_app_response

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
