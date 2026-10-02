from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="TotpLoginRequest")



@_attrs_define
class TotpLoginRequest:
    """ The second step of a sign-in with a second factor.

        Attributes:
            mfa_token (str): The ticket the password step answered.
            code (None | str | Unset): The app's six-digit code.
            recovery_code (None | str | Unset): Or one of the recovery codes, each good once.
     """

    mfa_token: str
    code: None | str | Unset = UNSET
    recovery_code: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        mfa_token = self.mfa_token

        code: None | str | Unset
        if isinstance(self.code, Unset):
            code = UNSET
        else:
            code = self.code

        recovery_code: None | str | Unset
        if isinstance(self.recovery_code, Unset):
            recovery_code = UNSET
        else:
            recovery_code = self.recovery_code


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "mfa_token": mfa_token,
        })
        if code is not UNSET:
            field_dict["code"] = code
        if recovery_code is not UNSET:
            field_dict["recovery_code"] = recovery_code

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        mfa_token = d.pop("mfa_token")

        def _parse_code(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        code = _parse_code(d.pop("code", UNSET))


        def _parse_recovery_code(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        recovery_code = _parse_recovery_code(d.pop("recovery_code", UNSET))


        totp_login_request = cls(
            mfa_token=mfa_token,
            code=code,
            recovery_code=recovery_code,
        )


        totp_login_request.additional_properties = d
        return totp_login_request

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
