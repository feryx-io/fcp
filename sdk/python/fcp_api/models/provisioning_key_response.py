from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="ProvisioningKeyResponse")



@_attrs_define
class ProvisioningKeyResponse:
    """ The tenant's provisioning URL.

        Attributes:
            key (str): The secret in the URL's path; anyone with it may fetch the tenant's
                phones' files (by MAC), so it is shown to administrators only.
            path (str): The path phones fetch under: `/provision/<key>/`.
            created_at (datetime.datetime):
            url (None | str | Unset): The whole URL, when the admin knows its public address
                (`[api.sso] public_url`).
     """

    key: str
    path: str
    created_at: datetime.datetime
    url: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        key = self.key

        path = self.path

        created_at = self.created_at.isoformat()

        url: None | str | Unset
        if isinstance(self.url, Unset):
            url = UNSET
        else:
            url = self.url


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "key": key,
            "path": path,
            "created_at": created_at,
        })
        if url is not UNSET:
            field_dict["url"] = url

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        key = d.pop("key")

        path = d.pop("path")

        created_at = datetime.datetime.fromisoformat(d.pop("created_at"))




        def _parse_url(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        url = _parse_url(d.pop("url", UNSET))


        provisioning_key_response = cls(
            key=key,
            path=path,
            created_at=created_at,
            url=url,
        )


        provisioning_key_response.additional_properties = d
        return provisioning_key_response

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
