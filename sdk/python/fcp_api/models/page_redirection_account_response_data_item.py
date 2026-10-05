from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.redirection_service import RedirectionService
from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="PageRedirectionAccountResponseDataItem")



@_attrs_define
class PageRedirectionAccountResponseDataItem:
    """ A tenant's account with a redirection service; the secret is never shown.

        Attributes:
            service (RedirectionService): A vendor's redirection service: a phone out of its box asks it where
                its provisioning server is.
            configured (bool):
            enabled (bool):
            carries_credentials (bool): Whether the service hands phones their own credentials.
            key_id (None | str | Unset): Yealink: the AccessKey ID; Poly: the API user id.
            account_id (None | str | Unset): Poly: the Reference ID devices are associated under.
            profile (None | str | Unset): Poly: the ZTP profile naming the tenant's provisioning URL.
            updated_at (datetime.datetime | None | Unset):
     """

    service: RedirectionService
    configured: bool
    enabled: bool
    carries_credentials: bool
    key_id: None | str | Unset = UNSET
    account_id: None | str | Unset = UNSET
    profile: None | str | Unset = UNSET
    updated_at: datetime.datetime | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        service = self.service.value

        configured = self.configured

        enabled = self.enabled

        carries_credentials = self.carries_credentials

        key_id: None | str | Unset
        if isinstance(self.key_id, Unset):
            key_id = UNSET
        else:
            key_id = self.key_id

        account_id: None | str | Unset
        if isinstance(self.account_id, Unset):
            account_id = UNSET
        else:
            account_id = self.account_id

        profile: None | str | Unset
        if isinstance(self.profile, Unset):
            profile = UNSET
        else:
            profile = self.profile

        updated_at: None | str | Unset
        if isinstance(self.updated_at, Unset):
            updated_at = UNSET
        elif isinstance(self.updated_at, datetime.datetime):
            updated_at = self.updated_at.isoformat()
        else:
            updated_at = self.updated_at


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "service": service,
            "configured": configured,
            "enabled": enabled,
            "carries_credentials": carries_credentials,
        })
        if key_id is not UNSET:
            field_dict["key_id"] = key_id
        if account_id is not UNSET:
            field_dict["account_id"] = account_id
        if profile is not UNSET:
            field_dict["profile"] = profile
        if updated_at is not UNSET:
            field_dict["updated_at"] = updated_at

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        service = RedirectionService(d.pop("service"))




        configured = d.pop("configured")

        enabled = d.pop("enabled")

        carries_credentials = d.pop("carries_credentials")

        def _parse_key_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        key_id = _parse_key_id(d.pop("key_id", UNSET))


        def _parse_account_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        account_id = _parse_account_id(d.pop("account_id", UNSET))


        def _parse_profile(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        profile = _parse_profile(d.pop("profile", UNSET))


        def _parse_updated_at(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                updated_at_type_0 = datetime.datetime.fromisoformat(data)



                return updated_at_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        updated_at = _parse_updated_at(d.pop("updated_at", UNSET))


        page_redirection_account_response_data_item = cls(
            service=service,
            configured=configured,
            enabled=enabled,
            carries_credentials=carries_credentials,
            key_id=key_id,
            account_id=account_id,
            profile=profile,
            updated_at=updated_at,
        )


        page_redirection_account_response_data_item.additional_properties = d
        return page_redirection_account_response_data_item

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
